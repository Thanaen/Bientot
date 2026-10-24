//! Tab and Shift+Tab reach the hours and the minutes of every time field, « Durée » and
//! « Motif » (#298): in « Déclarer du temps travaillé », in a correction and in the
//! schedule of « Paramètres »; Shift+Tab comes back to the first action of a bubble of the
//! week. It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use gpui::{App, HeadlessAppContext, WindowHandle, px};

use common::{press, type_text};
use jiff::tz::TimeZone;
use worktime_app::adjust::window::Opener;
use worktime_app::adjust::{AdjustForm, Intent, Seed, Stretch, Target, TimePart, Unrecorded};
use worktime_app::adjust_view::{self, AdjustView};
use worktime_app::demo::{self, DemoHost};
use worktime_app::settings::{Clock, Page, SettingsForm, UsagePingFacts};
use worktime_app::settings_view::{self, SettingsView};
use worktime_app::strings::Language;
use worktime_app::text_input;
use worktime_app::theme::size as token;
use worktime_app::week::SegmentKind;
use worktime_app::week_view::{self, WeekView};
use worktime_core::demo::DemoScenario;
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;
use worktime_domain::usage_ping::{Build, Channel};

fn bind_adjust_keys(cx: &mut App) {
    adjust_view::bind_keys(cx);
    text_input::bind_keys(cx);
}

/// An adjust window opened on `seed`, `prepare`d, with the focus on « Motif ».
fn adjust_window(
    cx: &mut HeadlessAppContext,
    seed: Seed,
    prepare: impl FnOnce(&mut AdjustForm),
) -> WindowHandle<AdjustView> {
    let target = Target {
        workday: LocalWorkday::new(jiff::civil::date(2026, 10, 8), "UTC"),
        zone: TimeZone::UTC,
    };
    let mut form = AdjustForm::new(
        target,
        seed,
        AdjustmentId::new("tab").unwrap(),
        // The evening of the day, so that the times of the day are in the past.
        "2026-10-08T18:00:00Z".parse().unwrap(),
        Language::French,
    );
    prepare(&mut form);
    let size = gpui::size(px(adjust_view::WINDOW_WIDTH), px(form.window_height()));
    let window = common::open(cx, size, |window, cx| {
        AdjustView::new(form, Opener::Week, window, cx)
    });
    window
        .update(cx, |view, window, cx| view.set_reason("", window, cx))
        .unwrap();
    window
}

/// The start and the end the window shows.
fn times(cx: &mut HeadlessAppContext, window: WindowHandle<AdjustView>) -> [String; 2] {
    window
        .update(cx, |view, _, _| {
            let form = view.form_mut();
            [form.start.text(), form.end.text()]
        })
        .unwrap()
}

/// Presses Enter and reads « Durée » and « Motif » as the save took them.
fn submit_and_read(cx: &mut HeadlessAppContext, window: WindowHandle<AdjustView>) -> [String; 2] {
    press(cx, window, "enter");
    window
        .update(cx, |view, _, _| {
            let form = view.form_mut();
            [form.duration.clone(), form.reason.clone()]
        })
        .expect("Enter closed the window: the focus was on « Annuler », which cancels")
}

#[test]
fn tab_reaches_the_times_and_the_reason_of_a_declaration() {
    let mut cx = common::app(false, bind_adjust_keys);
    let window = adjust_window(&mut cx, Seed::DeclareWork(None), |_| {});

    press(&mut cx, window, "shift-tab");
    type_text(&mut cx, window, "45");
    press(&mut cx, window, "shift-tab");
    // The hours, once typed, hand the focus to the minutes.
    type_text(&mut cx, window, "17");
    press(&mut cx, window, "shift-tab");
    press(&mut cx, window, "shift-tab");
    type_text(&mut cx, window, "30");
    press(&mut cx, window, "shift-tab");
    type_text(&mut cx, window, "08");
    assert_eq!(times(&mut cx, window), ["08:30", "17:45"].map(String::from));
    for _ in 0..3 {
        press(&mut cx, window, "tab");
    }
    type_text(&mut cx, window, "Oubli");

    assert_eq!(
        submit_and_read(&mut cx, window),
        ["", "Oubli"].map(String::from)
    );
}

/// Shift+Tab from « Motif » passes the times, then reaches the slots of unrecorded time:
/// Space on one moves Début and Fin to it.
#[test]
fn shift_tab_reaches_the_slots_of_unrecorded_time() {
    let mut cx = common::app(false, bind_adjust_keys);
    let slot = |start: &str, end: &str| {
        UtcInterval::new(
            format!("2026-10-08T{start}:00Z").parse().unwrap(),
            format!("2026-10-08T{end}:00Z").parse().unwrap(),
        )
        .unwrap()
    };
    let unrecorded = Unrecorded::new(
        vec![slot("09:40", "09:46"), slot("14:00", "14:20")],
        vec![Stretch {
            kind: SegmentKind::Work,
            interval: slot("09:00", "16:00"),
        }],
    )
    .unwrap();
    let window = adjust_window(&mut cx, Seed::DeclareWork(Some(unrecorded)), |_| {});
    assert_eq!(times(&mut cx, window), ["14:00", "14:20"].map(String::from));

    for _ in 0..6 {
        press(&mut cx, window, "shift-tab");
    }
    press(&mut cx, window, "space");
    assert_eq!(times(&mut cx, window), ["09:40", "09:46"].map(String::from));
}

#[test]
fn tab_reaches_the_duration_and_the_reason_of_a_correction() {
    let mut cx = common::app(false, bind_adjust_keys);
    let window = adjust_window(&mut cx, Seed::Blank, |form| {
        form.set_intent(Intent::AddDuration)
    });

    press(&mut cx, window, "shift-tab");
    type_text(&mut cx, window, "90");
    press(&mut cx, window, "tab");
    type_text(&mut cx, window, "Oubli");

    assert_eq!(
        submit_and_read(&mut cx, window),
        ["90", "Oubli"].map(String::from)
    );
}

/// Presses Tab past the pages and the targets until ↑ steps the hours of the lunch start, and
/// takes each step back.
fn tab_to_the_lunch_start_hours(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>) {
    let hours = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| {
            let form = window.read(cx).unwrap().form();
            form.time(Clock::LunchStart).value(TimePart::Hours)
        })
    };
    let before = hours(cx);
    for _ in 0..30 {
        press(cx, window, "tab");
        press(cx, window, "up");
        let stepped = hours(cx) != before;
        press(cx, window, "down");
        if stepped {
            assert_eq!(hours(cx), before, "↓ does not undo ↑");
            return;
        }
    }
    panic!("30 presses of Tab do not reach the hours of the lunch start");
}

#[test]
fn tab_reaches_the_times_of_the_schedule() {
    let mut cx = common::app(false, settings_view::bind_keys);
    let usage_ping = UsagePingFacts {
        build: Build::new("1.0.0", Channel::Store).unwrap(),
        released: None,
        today: jiff::civil::date(2026, 10, 13),
        last_sent: None,
    };
    let mut form = SettingsForm::new(Default::default(), usage_ping, Language::French);
    form.go_to(Page::Schedule);
    let size = gpui::size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT);
    let window = common::open(&mut cx, size, |window, cx| {
        SettingsView::new(form, None, window, cx)
    });
    let schedule_times = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| {
            let form = window.read(cx).unwrap().form();
            [Clock::LunchStart, Clock::LunchEnd, Clock::Departure]
                .map(|clock| form.time(clock).text())
        })
    };
    assert_eq!(
        schedule_times(&mut cx),
        ["12:00", "14:00", "18:30"].map(String::from)
    );

    tab_to_the_lunch_start_hours(&mut cx, window);
    type_text(&mut cx, window, "1145");
    press(&mut cx, window, "tab");
    type_text(&mut cx, window, "13");
    press(&mut cx, window, "tab");
    press(&mut cx, window, "tab");
    type_text(&mut cx, window, "15");
    press(&mut cx, window, "shift-tab");
    type_text(&mut cx, window, "17");

    assert_eq!(
        schedule_times(&mut cx),
        ["11:45", "13:00", "17:15"].map(String::from)
    );
}

#[test]
fn shift_tab_comes_back_to_the_first_action_of_a_bubble() {
    let mut cx = common::app(false, week_view::bind_keys);
    let directory = demo::throwaway_directory();
    let week = DemoHost::open(DemoScenario::HistoryWeek, &directory).and_then(|mut host| {
        host.refresh(Language::French)?;
        host.week()
    });
    std::fs::remove_dir_all(&directory).ok();
    let week = week.unwrap();
    let size = gpui::size(token::WEEK_WINDOW_WIDTH, px(800.));
    let window = common::open(&mut cx, size, |window, cx| {
        WeekView::new(Some(week), Language::French, window, cx)
    });
    let focused = |cx: &mut HeadlessAppContext| {
        cx.update_window(window.into(), |_, window, cx| window.focused(cx))
            .unwrap()
    };

    press(&mut cx, window, "n");
    let bubble_open = window
        .update(&mut cx, |view, _, _| {
            view.state_mut().unwrap().bubble().is_some()
        })
        .unwrap();
    assert!(bubble_open, "N opens no bubble in the history week");
    let first = focused(&mut cx);
    press(&mut cx, window, "tab");
    assert_ne!(focused(&mut cx), first, "Tab stays on the first action");
    press(&mut cx, window, "shift-tab");
    assert_eq!(
        focused(&mut cx),
        first,
        "Shift+Tab from the second action skips the first"
    );
}
