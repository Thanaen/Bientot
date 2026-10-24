//! The flyout's source hint offers to count unrecorded time as work or to see it in the week,
//! and the settings for a failed source, as `MainWindow.OnSourceActionClick` of the .NET app;
//! the notice of restored settings offers the same settings. Each control is reached with Tab and pressed with
//! Enter, as a keyboard user does. It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, FocusHandle, HeadlessAppContext, KeyDownEvent, KeyUpEvent, Keystroke,
    PlatformInput, WindowHandle, px, size,
};
use worktime_app::assets::{self, Assets};
use worktime_app::demo::{self, DemoHost};
use worktime_app::flyout::{
    self, AdjustTime, Alerts, CountUnrecorded, FinishDay, Flyout, OpenGeneralSettings,
    OpenSettings, OpenWeek, Refresh, ShowUnrecorded, TogglePause, UseBuiltInDetection,
};
use worktime_app::live::Tracking;
use worktime_app::strings::Language;
use worktime_app::theme::ThemeMode;
use worktime_app::theme::size::FLYOUT_WIDTH;
use worktime_app::widgets::Notice;
use worktime_app::workday::{DayView, SourceHint};
use worktime_core::demo::DemoScenario;
use worktime_domain::accounting::TrackingState;
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;

type Pressed = Rc<RefCell<Vec<&'static str>>>;

fn day(scenario: DemoScenario) -> DayView {
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(scenario, &directory)
        .unwrap()
        .refresh(Language::French);
    std::fs::remove_dir_all(&directory).ok();
    day.unwrap()
}

fn record<A: gpui::Action>(pressed: &Pressed, name: &'static str, cx: &mut App) {
    let pressed = pressed.clone();
    cx.on_action(move |_: &A, _| pressed.borrow_mut().push(name));
}

fn press(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, key: &str) {
    let keystroke = Keystroke::parse(key).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
        window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
        window.refresh();
    })
    .unwrap();
    cx.run_until_parked();
}

fn focused(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> Option<FocusHandle> {
    cx.update_window(window.into(), |_, window, cx| window.focused(cx))
        .unwrap()
}

/// What Enter does on each Tab stop of the flyout of `scenario`, in Tab order.
fn tab_stops(scenario: DemoScenario, tracking: Tracking) -> Vec<&'static str> {
    tab_stops_with(scenario, tracking, Alerts::default())
}

/// [`tab_stops`] of a flyout that opens on `alerts`.
fn tab_stops_with(scenario: DemoScenario, tracking: Tracking, alerts: Alerts) -> Vec<&'static str> {
    press_every_tab_stop(day(scenario), tracking, alerts).0
}

/// What the hint's two buttons sent.
#[derive(Debug, Default, PartialEq)]
struct Sent {
    counts: Vec<CountUnrecorded>,
    shown: Vec<ShowUnrecorded>,
}

/// Enter on each Tab stop of the flyout of `day`, in Tab order: what each one did, and what the
/// buttons of unrecorded time sent.
fn press_every_tab_stop(
    day: DayView,
    tracking: Tracking,
    alerts: Alerts,
) -> (Vec<&'static str>, Sent) {
    let pressed = Pressed::default();
    let sent = Rc::new(RefCell::new(Sent::default()));
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_asset_source(text_system, Arc::new(Assets));
    cx.update(|cx| {
        cx.set_global(ThemeMode::Light);
        cx.set_global(Language::French);
        assets::load_fonts(cx).unwrap();
        flyout::bind_keys(cx);
        record::<OpenWeek>(&pressed, "open week", cx);
        record::<OpenSettings>(&pressed, "open settings", cx);
        record::<OpenGeneralSettings>(&pressed, "open general settings", cx);
        record::<UseBuiltInDetection>(&pressed, "use built-in detection", cx);
        record::<Refresh>(&pressed, "refresh", cx);
        record::<TogglePause>(&pressed, "toggle pause", cx);
        record::<AdjustTime>(&pressed, "adjust time", cx);
        record::<FinishDay>(&pressed, "finish day", cx);
        // One listener only: the last one registered for an action is the one that runs.
        let (counted, counts) = (pressed.clone(), sent.clone());
        cx.on_action(move |action: &CountUnrecorded, _| {
            counted.borrow_mut().push("count unrecorded");
            counts.borrow_mut().counts.push(action.clone());
        });
        let (shown, shows) = (pressed.clone(), sent.clone());
        cx.on_action(move |action: &ShowUnrecorded, _| {
            shown.borrow_mut().push("show unrecorded");
            shows.borrow_mut().shown.push(action.clone());
        });
    });
    let window = cx
        .open_window(size(FLYOUT_WIDTH, px(800.)), |window, cx| {
            cx.new(|cx| Flyout::new(day, tracking, None, window, cx).with_alerts(alerts))
        })
        .unwrap();
    press(&mut cx, window, "tab");
    let first = focused(&mut cx, window).expect("Tab focuses something");
    loop {
        press(&mut cx, window, "enter");
        press(&mut cx, window, "tab");
        if focused(&mut cx, window).as_ref() == Some(&first) {
            return (pressed.take(), sent.take());
        }
        assert!(
            pressed.borrow().len() < 32,
            "Tab comes back to where it started"
        );
    }
}

fn running(working_day: bool) -> Tracking {
    Tracking::new(TrackingState::Running, working_day)
}

#[test]
fn unrecorded_time_offers_to_count_it_or_see_it_in_the_week() {
    assert_eq!(
        tab_stops(DemoScenario::ProblemPeriods, running(true)),
        [
            "open week",
            "open settings",
            "count unrecorded",
            "show unrecorded",
            "toggle pause",
            "adjust time",
            "finish day",
        ]
    );
}

/// As `UpdateCommands` of the .NET app enables the action only when time can be added; the
/// week still shows it.
#[test]
fn unrecorded_time_on_a_day_off_cannot_be_counted() {
    assert_eq!(
        tab_stops(DemoScenario::ProblemPeriods, running(false)),
        ["open week", "open settings", "show unrecorded"]
    );
}

#[test]
fn a_failed_built_in_source_offers_its_settings_and_a_refresh() {
    assert_eq!(
        tab_stops(DemoScenario::Unavailable, running(true)),
        [
            "open week",
            "open settings",
            "open general settings",
            "refresh",
            "toggle pause",
            "adjust time",
            "finish day",
        ]
    );
}

/// The settings would only repeat the switch to the built-in detection the hint offers.
#[test]
fn activitywatch_in_trouble_offers_the_built_in_detection_and_a_refresh() {
    assert_eq!(
        tab_stops(DemoScenario::ActivityWatchUnavailable, running(true)),
        [
            "open week",
            "open settings",
            "use built-in detection",
            "refresh",
            "toggle pause",
            "adjust time",
            "finish day",
        ]
    );
}

#[test]
fn a_healthy_day_has_no_source_hint() {
    assert_eq!(
        tab_stops(DemoScenario::OnTrack, running(true)),
        [
            "open week",
            "open settings",
            "toggle pause",
            "adjust time",
            "finish day",
        ]
    );
}

/// The notice of restored settings opens them as the hint of a failed source does; the other
/// warnings of the start offer nothing.
#[test]
fn restored_settings_offer_the_settings() {
    let alerts = Alerts {
        announcements: vec![
            Notice::settings_recovered("Les valeurs par défaut sont rétablies."),
            Notice::warning("Les notifications Windows ne sont pas disponibles."),
        ],
        refresh_failure: None,
    };
    assert_eq!(
        tab_stops_with(DemoScenario::OnTrack, running(true), alerts),
        [
            "open week",
            "open settings",
            "open general settings",
            "toggle pause",
            "adjust time",
            "finish day",
        ]
    );
}

fn interval(start: &str, end: &str) -> UtcInterval {
    UtcInterval::new(start.parse().unwrap(), end.parse().unwrap()).unwrap()
}

/// « Oui, compter 26 min » counts the two unrecorded intervals of the problem periods' day, and
/// « Voir sur la frise » shows them in the week.
#[test]
fn the_hint_s_buttons_send_the_unrecorded_time_of_its_day() {
    let day = day(DemoScenario::ProblemPeriods);
    let Some(SourceHint::Unrecorded { message, .. }) = &day.source_hint else {
        panic!("the problem periods leave unrecorded time");
    };
    assert_eq!(
        message,
        "Temps non renseigné · 26\u{a0}min en 2\u{a0}créneaux, entre 09:40 et 14:20. \
         Travailliez-vous\u{202f}?"
    );
    let unrecorded = vec![
        interval("2026-08-31T09:40:12Z", "2026-08-31T09:45:42Z"),
        interval("2026-08-31T14:00:00Z", "2026-08-31T14:20:00Z"),
    ];
    let sent = press_every_tab_stop(day, running(true), Alerts::default()).1;
    assert_eq!(
        sent,
        Sent {
            counts: vec![CountUnrecorded {
                workday: LocalWorkday::new(jiff::civil::date(2026, 8, 31), "UTC"),
                intervals: unrecorded.clone(),
            }],
            shown: vec![ShowUnrecorded {
                date: jiff::civil::date(2026, 8, 31),
                intervals: unrecorded,
            }],
        }
    );
}
