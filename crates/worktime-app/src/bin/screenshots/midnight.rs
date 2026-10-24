//! A slot that closes its day, as Friday evening of a launch on Saturday leaves one:
//! `week-midnight-gap-bubble.png`, the bubble of the gap from 22:00 to 24:00;
//! `adjust-declare-work-midnight.png`, the declaration « Déclarer du temps travaillé… » opens
//! on it, whose « Horaires proposés » end at 24:00; and `week-midnight-declared.png`, the week
//! once the gap is declared, with the result.

use anyhow::Context as _;
use gpui::{AppContext as _, HeadlessAppContext, size};
use jiff::civil::Time;
use worktime_app::adjust::window::Opener;
use worktime_app::demo;
use worktime_app::live::Tracking;
use worktime_app::strings::Language;
use worktime_app::theme::size as token;
use worktime_app::week::{Segment, SegmentKind, WeekState};
use worktime_app::week_view::WeekView;
use worktime_domain::accounting::TrackingState;

use super::capture;
use super::fixtures::{Behind, on_saturday};
use super::shared::{adjust_form, open_adjust, open_segment};

/// Friday's gap left to settle, which runs to the midnight closing the day.
fn to_midnight(segment: &Segment) -> bool {
    segment.kind == SegmentKind::Gap
        && segment.needs_action
        && segment.local_end.time() == Time::midnight()
}

pub fn capture_midnight(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let directory = demo::throwaway_directory();
    let read = on_saturday(&directory, Behind::WorkedToMidnight, |host| {
        let before = host.week()?;
        let mut state = WeekState::new(before.clone(), language);
        let gap = state
            .timeline
            .days
            .iter()
            .find_map(|day| {
                let segment = day.segments.iter().find(|segment| to_midnight(segment))?;
                Some(segment.id(day.date))
            })
            .context("Friday evening leaves a gap to midnight")?;
        state.select(gap.date);
        let (workday, seed) = state
            .declare_work()
            .context("the gap takes a declaration")?;
        let form = adjust_form(workday, seed, host.now(), language);
        let mut saved = form.clone();
        let declared = saved.submit().context("the gap is declared as it opens")?;
        host.save_adjustment(&declared)
            .map_err(|key| anyhow::anyhow!("cannot declare the gap: {key:?}"))?;
        host.refresh();
        let closed = saved.finished(Ok(true)).context("the declaration saves")?;
        Ok((before, form, host.week()?, closed))
    });
    std::fs::remove_dir_all(&directory).ok();
    let (before, form, after, closed) = read?;

    let window = cx.open_window(
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| {
            cx.new(|cx| {
                let mut view = WeekView::new(Some(before), language, window, cx);
                view.set_tracking(Tracking::new(TrackingState::Running, false));
                view
            })
        },
    )?;
    open_segment(cx, window, "a gap to midnight", to_midnight)?;
    capture(
        cx,
        window.into(),
        &output.join("week-midnight-gap-bubble.png"),
    )?;
    let declaration = open_adjust(cx, form, Opener::Week)?;
    capture(
        cx,
        declaration.into(),
        &output.join("adjust-declare-work-midnight.png"),
    )?;
    cx.update(|cx| {
        window.update(cx, |view, _, cx| {
            if let Some(state) = view.state_mut() {
                // As « Déclarer du temps travaillé… » leaves it once the window opened.
                state.close_bubble();
                state.reload(after, true);
                state.adjusted(closed);
            }
            cx.notify();
        })
    })?;
    capture(
        cx,
        window.into(),
        &output.join("week-midnight-declared.png"),
    )
}
