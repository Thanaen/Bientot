//! The week of the scenarios that store one, its palette and its bubbles:
//! - `week-<scenario>.png`, the week;
//! - `week-<scenario>-narrow.png`, the week at its minimum width, as Windows Snap leaves it;
//! - `palette-week.png`, the command palette over the history week;
//! - `week-<scenario>-bubble.png`, the bubble of its first slot to settle;
//! - `week-history-week-correction-bubble.png`, the bubble of the history week's meeting;
//! - `week-history-week-gap-bubble.png`, the bubble of Wednesday's gap in the history week;
//! - `adjust-declare-work-week.png`, « Déclarer du temps travaillé… » of that gap's day;
//! - `week-history-week-<bound|locked>-bubble.png`, the bubble of its long lock once answered;
//! - `week-range-<work|extended|lock|gaps>.png`, ranges selected on the timeline: Wednesday's
//!   work at 12:00, then stretched over the gap after it while its handle is dragged, then
//!   back to the lock before it, and Monday's gaps in the problem periods;
//! - `week-range-unrecorded.png`, the unrecorded time of the problem periods as « Voir sur la
//!   frise » of the flyout selects its first gap, with the note that links it to the flyout's
//!   total;
//! - `week-range-work-motion-<0|8|17|50|100>.png`, the entrance of the popover a click opens;
//! - `week-proposal.png`, the result bar of a correction made the third Wednesday in a row,
//!   which offers to make it an appointment;
//! - `week-appointments.png`, the brackets of three recurring appointments: Monday's cut at the
//!   axis, Thursday's over the sleep it turned into an absence to review and Friday's cut at now;
//!   `-tooltip` with the pointer on Friday's, `-bubble` with Thursday's absence open;
//!   `-tooltip-merged` with the pointer on the one bracket of two appointments whose hours
//!   overlap on Friday.

use anyhow::Context as _;
use gpui::{
    AppContext as _, HeadlessAppContext, Modifiers, MouseMoveEvent, PlatformInput, WindowHandle,
    point, px, size,
};
use jiff::civil::{Weekday, time};
use worktime_app::adjust::window::Opener;
use worktime_app::demo::{self, DemoHost};
use worktime_app::palette_view;
use worktime_app::strings::Language;
use worktime_app::theme::{motion, size as token};
use worktime_app::week::{Answer, Edge, Recorded, SegmentKind, WeekTimeline};
use worktime_app::week_view::WeekView;
use worktime_app::workday::SourceHint;
use worktime_core::demo::DemoScenario;
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::configuration::{
    AppointmentProposal, Occurrence, RecurringAppointment, WorkTimeSettings,
};

use super::shared::{adjust_form, demo_day, open_adjust, open_segment, read_demo_week};
use super::{HISTORY_WEEK_MEETING, MENU_FRAMES, capture, capture_entrance};

pub fn capture_weeks(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // The scenarios that store a week; the others show an empty one.
    for scenario in [
        DemoScenario::HistoryWeek,
        DemoScenario::ProblemPeriods,
        DemoScenario::FragmentedDay,
    ] {
        let directory = demo::throwaway_directory();
        let read = DemoHost::open(scenario, &directory).and_then(|mut host| {
            host.refresh(language)?;
            Ok((host.week()?, host.correction_time()))
        });
        std::fs::remove_dir_all(&directory).ok();
        let (week, now) = read?;
        let slug = demo::slug(scenario);
        let narrow = cx.open_window(
            size(token::WEEK_WINDOW_MIN_WIDTH, token::WEEK_WINDOW_HEIGHT),
            |window, cx| cx.new(|cx| WeekView::new(Some(week.clone()), language, window, cx)),
        )?;
        capture(
            cx,
            narrow.into(),
            &output.join(format!("week-{slug}-narrow.png")),
        )?;
        let window = cx.open_window(
            size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
            |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
        )?;
        capture(cx, window.into(), &output.join(format!("week-{slug}.png")))?;
        if scenario == DemoScenario::HistoryWeek {
            cx.update(|cx| window.update(cx, |view, window, cx| view.open_palette(window, cx)))?;
            capture(cx, window.into(), &output.join("palette-week.png"))?;
            cx.update(|cx| {
                window.update(cx, |_, window, cx| {
                    window.dispatch_action(Box::new(palette_view::Dismiss), cx)
                })
            })?;
        }
        // The bubble of the first slot to settle, as `N` opens it.
        let opened = cx.update(|cx| {
            window.update(cx, |view, _, cx| {
                let state = view.state_mut()?;
                state.settle_next();
                cx.notify();
                state.bubble().map(|_| ())
            })
        })?;
        if opened.is_some() {
            capture(
                cx,
                window.into(),
                &output.join(format!("week-{slug}-bubble.png")),
            )?;
            // The palette opened over a bubble closes it, or the bubble would draw on top.
            let covered = cx.update(|cx| {
                window.update(cx, |view, window, cx| {
                    view.open_palette(window, cx);
                    view.state_mut()
                        .is_some_and(|state| state.bubble().is_some())
                })
            })?;
            anyhow::ensure!(!covered, "the palette left the {slug} bubble open");
            cx.update(|cx| {
                window.update(cx, |_, window, cx| {
                    window.dispatch_action(Box::new(palette_view::Dismiss), cx)
                })
            })?;
        }
        // The bubble of the meeting the demo declared, which offers « Modifier… ».
        if scenario == DemoScenario::HistoryWeek {
            let meeting = AdjustmentId::new(HISTORY_WEEK_MEETING)?;
            open_segment(cx, window, "its meeting", |segment| {
                segment.adjustment_id.as_ref() == Some(&meeting)
            })?;
            capture(
                cx,
                window.into(),
                &output.join(format!("week-{slug}-correction-bubble.png")),
            )?;
            capture_declarations(cx, window, slug, now, language, output)?;
        }
    }
    capture_answered_locks(cx, language, output)?;
    capture_proposal(cx, language, output)?;
    capture_appointments(cx, language, output)?;
    capture_ranges(cx, language, output)
}

/// The history week with a recurring appointment on Monday from 7:00 to 9:00, before the axis,
/// on Thursday from 11:00 to 12:00, over its sleep, and on Friday from 17:00 to 19:30, past now;
/// then with two on Friday whose hours overlap, under one bracket.
fn capture_appointments(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let week = appointments_week(
        language,
        vec![
            appointment(
                "monday",
                "",
                Weekday::Monday,
                time(7, 0, 0, 0),
                time(9, 0, 0, 0),
            ),
            appointment(
                "thursday",
                "Réunion d’équipe",
                Weekday::Thursday,
                time(11, 0, 0, 0),
                time(12, 0, 0, 0),
            ),
            appointment(
                "friday",
                "",
                Weekday::Friday,
                time(17, 0, 0, 0),
                time(19, 30, 0, 0),
            ),
        ],
    )?;
    let window = cx.open_window(
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
    )?;
    capture(cx, window.into(), &output.join("week-appointments.png"))?;
    point_at_friday_bracket(cx, window)?;
    capture(
        cx,
        window.into(),
        &output.join("week-appointments-tooltip.png"),
    )?;
    open_segment(
        cx,
        window,
        "Thursday's absence in its appointment",
        |segment| segment.is_appointment() && segment.local_start.date() == demo_day(3),
    )?;
    capture(
        cx,
        window.into(),
        &output.join("week-appointments-bubble.png"),
    )?;

    let week = appointments_week(
        language,
        vec![
            appointment(
                "client",
                "Point client",
                Weekday::Friday,
                time(17, 0, 0, 0),
                time(18, 0, 0, 0),
            ),
            appointment(
                "sprint",
                "Revue de sprint",
                Weekday::Friday,
                time(17, 30, 0, 0),
                time(19, 30, 0, 0),
            ),
        ],
    )?;
    let window = cx.open_window(
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
    )?;
    point_at_friday_bracket(cx, window)?;
    capture(
        cx,
        window.into(),
        &output.join("week-appointments-tooltip-merged.png"),
    )
}

fn appointment(
    id: &str,
    name: &str,
    weekday: Weekday,
    start: jiff::civil::Time,
    end: jiff::civil::Time,
) -> RecurringAppointment {
    RecurringAppointment {
        id: id.to_owned(),
        name: name.to_owned(),
        weekday,
        start,
        end,
        enabled: true,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    }
}

/// The history week once `appointments` are the recurring ones.
fn appointments_week(
    language: Language,
    appointments: Vec<RecurringAppointment>,
) -> anyhow::Result<WeekTimeline> {
    let directory = demo::throwaway_directory();
    let read = DemoHost::open(DemoScenario::HistoryWeek, &directory).and_then(|mut host| {
        let settings = WorkTimeSettings {
            recurring_appointments: appointments,
            ..host.settings().clone()
        };
        host.save_settings(&settings)?;
        host.refresh(language)?;
        host.week()
    });
    std::fs::remove_dir_all(&directory).ok();
    read
}

/// Rests the pointer on the middle of the first bracket of today, Friday, the open day.
fn point_at_friday_bracket(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<WeekView>,
) -> anyhow::Result<()> {
    let pointer = cx.update(|cx| {
        window.update(cx, |view, _, _| {
            let bounds = view.strip_bounds()?;
            let state = view.state_mut()?;
            let friday = state.page().days.into_iter().find(|row| row.is_today)?;
            let marker = friday.markers.first()?;
            let width = f32::from(bounds.size.width);
            Some(point(
                bounds.origin.x + px((marker.start + marker.length / 2.) * width),
                bounds.origin.y
                    - token::APPOINTMENT_MARKER_GAP_FOCUSED
                    - token::APPOINTMENT_MARKER_HEIGHT / 2.,
            ))
        })
    })?;
    let pointer = pointer.context("Friday draws its appointment")?;
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position: pointer,
                pressed_button: None,
                modifiers: Modifiers::default(),
            }),
            cx,
        );
    })?;
    cx.run_until_parked();
    Ok(())
}

/// Wednesday's long lock counted as work, its result offering « Compter chaque mercredi… » as
/// if the two Wednesdays before had the same correction.
fn capture_proposal(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let week = read_demo_week(DemoScenario::HistoryWeek, language, &[])?;
    let answered = read_demo_week(DemoScenario::HistoryWeek, language, &[Answer::Count])?;
    let window = cx.open_window(
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
    )?;
    let hours = (time(10, 0, 0, 0), time(12, 0, 0, 0));
    let occurrence = |date| Occurrence {
        date,
        start: hours.0,
        end: hours.1,
    };
    let latest = demo_day(2);
    let proposal = AppointmentProposal {
        weekday: Weekday::Wednesday,
        start: hours.0,
        end: hours.1,
        alternate_weeks_from: None,
        occurrences: [14, 7, 0]
            .map(|days| occurrence(latest - jiff::Span::new().days(days)))
            .to_vec(),
    };
    let offered = cx.update(|cx| {
        window.update(cx, |view, _, cx| {
            let state = view.state_mut()?;
            state.settle_next();
            state.choose(Answer::Count)?;
            state.reload(answered, true);
            state.finished(Ok(Recorded {
                saved: Vec::new(),
                recalculated: true,
                proposal: Some(proposal),
            }));
            cx.notify();
            state.info_bar()?.offer.map(|_| ())
        })
    })?;
    offered.context("the result bar offers the appointment")?;
    capture(cx, window.into(), &output.join("week-proposal.png"))
}

/// Ranges selected on the timeline, as a click, a drag and the keyboard leave them.
fn capture_ranges(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let open = |cx: &mut HeadlessAppContext, scenario| -> anyhow::Result<WindowHandle<WeekView>> {
        let week = read_demo_week(scenario, language, &[])?;
        cx.open_window(
            size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
            |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
        )
    };
    let change = |cx: &mut HeadlessAppContext,
                  window: WindowHandle<WeekView>,
                  change: &dyn Fn(&mut WeekView)|
     -> anyhow::Result<()> {
        cx.update(|cx| {
            window.update(cx, |view, _, cx| {
                change(view);
                cx.notify();
            })
        })?;
        Ok(())
    };
    let window = open(cx, DemoScenario::HistoryWeek)?;
    let wednesday = demo_day(2);
    open_segment(cx, window, "Wednesday's work at 12:00", |segment| {
        segment.local_start.date() == wednesday && segment.start_time() == "12:00"
    })?;
    capture(cx, window.into(), &output.join("week-range-work.png"))?;
    change(cx, window, &|view| view.replay_bubble_entrance())?;
    capture_entrance(
        cx,
        window.into(),
        motion::MENU,
        MENU_FRAMES,
        output,
        "week-range-work",
    )?;
    change(cx, window, &|view| {
        if let Some(state) = view.state_mut() {
            state.extend(Edge::End);
        }
        view.set_dragging(Some(Edge::End));
    })?;
    capture(cx, window.into(), &output.join("week-range-extended.png"))?;
    change(cx, window, &|view| {
        view.set_dragging(None);
        if let Some(state) = view.state_mut() {
            state.nudge(Edge::End, -1);
            state.nudge(Edge::End, -1);
            state.nudge(Edge::End, -1);
            state.nudge(Edge::Start, -1);
        }
    })?;
    capture(cx, window.into(), &output.join("week-range-lock.png"))?;

    let window = open(cx, DemoScenario::ProblemPeriods)?;
    open_segment(cx, window, "a gap to settle", |segment| {
        segment.kind == SegmentKind::Gap && segment.needs_action
    })?;
    change(cx, window, &|view| {
        if let Some(state) = view.state_mut() {
            for _ in 0..4 {
                state.extend(Edge::End);
            }
        }
    })?;
    capture(cx, window.into(), &output.join("week-range-gaps.png"))?;

    let directory = demo::throwaway_directory();
    let hint = DemoHost::open(DemoScenario::ProblemPeriods, &directory)
        .and_then(|mut host| host.refresh(language))
        .map(|day| day.source_hint);
    std::fs::remove_dir_all(&directory).ok();
    let Some(SourceHint::Unrecorded {
        workday, intervals, ..
    }) = hint?
    else {
        anyhow::bail!("the problem periods leave time unrecorded");
    };
    let window = open(cx, DemoScenario::ProblemPeriods)?;
    change(cx, window, &|view| {
        if let Some(state) = view.state_mut() {
            state.show_unrecorded(workday.date, &intervals);
        }
    })?;
    capture(cx, window.into(), &output.join("week-range-unrecorded.png"))
}

/// The bubble of Wednesday's long lock once answered: excluded, its correction stays bound to
/// its review; confirmed, it is locked.
fn capture_answered_locks(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for (answer, name) in [(Answer::Exclude, "bound"), (Answer::Count, "locked")] {
        let week = read_demo_week(DemoScenario::HistoryWeek, language, &[answer])?;
        let window = cx.open_window(
            size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
            |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
        )?;
        let wednesday = demo_day(2);
        open_segment(cx, window, "Wednesday's answered lock", |segment| {
            segment.local_start.date() == wednesday && segment.adjustment_id.is_some()
        })?;
        capture(
            cx,
            window.into(),
            &output.join(format!("week-history-week-{name}-bubble.png")),
        )?;
    }
    Ok(())
}

/// Wednesday's gap in the history week and its bubble, then « Déclarer du temps travaillé… »
/// of its day.
fn capture_declarations(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<WeekView>,
    slug: &str,
    now: jiff::Timestamp,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    open_segment(cx, window, "a gap to settle", |segment| {
        segment.kind == SegmentKind::Gap && segment.needs_action
    })?;
    let bubble = output.join(format!("week-{slug}-gap-bubble.png"));
    capture(cx, window.into(), &bubble)?;
    let seeds = cx.update(|cx| {
        window.update(cx, |view, _, _| {
            let state = view.state_mut()?;
            Some([("adjust-declare-work-week", state.declare_work()?)])
        })
    })?;
    for (name, (workday, seed)) in seeds.context("the gap's day takes a declaration")? {
        let window = open_adjust(cx, adjust_form(workday, seed, now, language), Opener::Week)?;
        capture(cx, window.into(), &output.join(format!("{name}.png")))?;
    }
    Ok(())
}
