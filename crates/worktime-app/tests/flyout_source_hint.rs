//! The flyout's source hint offers what `MainWindow.OnSourceActionClick` of the .NET app does:
//! recording the work of unrecorded time, the settings for a failed source; the notice of
//! restored settings offers the same settings. Each control is reached with Tab and pressed with
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
    self, AdjustTime, Alerts, DeclareWork, FinishDay, Flyout, OpenGeneralSettings, OpenSettings,
    OpenWeek, Refresh, TogglePause, UseBuiltInDetection,
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

/// Enter on each Tab stop of the flyout of `day`, in Tab order: what each one did, and every
/// declaration of work it sent.
fn press_every_tab_stop(
    day: DayView,
    tracking: Tracking,
    alerts: Alerts,
) -> (Vec<&'static str>, Vec<DeclareWork>) {
    let pressed = Pressed::default();
    let declared = Rc::new(RefCell::new(Vec::new()));
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
        let (pressed, declared) = (pressed.clone(), declared.clone());
        cx.on_action(move |action: &DeclareWork, _| {
            pressed.borrow_mut().push("declare work");
            declared.borrow_mut().push(action.clone());
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
            return (pressed.take(), declared.take());
        }
        assert!(
            pressed.borrow().len() < 32,
            "Tab comes back to where it started"
        );
    }
}

/// The unrecorded intervals of the hint of `day`.
fn unrecorded(day: &DayView) -> Vec<UtcInterval> {
    match &day.source_hint {
        Some(SourceHint::Unrecorded { intervals, .. }) => intervals.clone(),
        hint => panic!("the day has no unrecorded time: {hint:?}"),
    }
}

/// What « Renseigner un créneau travaillé » declares on a day whose unrecorded time is
/// `intervals`.
fn declared_on(intervals: Vec<UtcInterval>) -> Vec<DeclareWork> {
    let mut day = day(DemoScenario::ProblemPeriods);
    let Some(SourceHint::Unrecorded {
        intervals: shown, ..
    }) = &mut day.source_hint
    else {
        panic!("the problem periods leave unrecorded time");
    };
    *shown = intervals;
    press_every_tab_stop(day, running(true), Alerts::default()).1
}

fn running(working_day: bool) -> Tracking {
    Tracking::new(TrackingState::Running, working_day)
}

#[test]
fn unrecorded_time_offers_to_record_the_work() {
    assert_eq!(
        tab_stops(DemoScenario::ProblemPeriods, running(true)),
        [
            "open week",
            "open settings",
            "declare work",
            "toggle pause",
            "adjust time",
            "finish day",
        ]
    );
}

/// As `UpdateCommands` of the .NET app enables the action only when time can be added.
#[test]
fn unrecorded_time_on_a_day_off_cannot_be_recorded() {
    assert_eq!(
        tab_stops(DemoScenario::ProblemPeriods, running(false)),
        ["open week", "open settings"]
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

/// The declaration opens on the unrecorded time, in its day, as the hint reports it.
fn declared_slots(intervals: Vec<UtcInterval>) -> Vec<Vec<UtcInterval>> {
    declared_on(intervals)
        .into_iter()
        .map(|DeclareWork(unrecorded)| {
            unrecorded
                .expect("a declaration on the unrecorded time")
                .slots()
                .to_vec()
        })
        .collect()
}

/// As `MainWindow.OnSourceActionClick` of the .NET app: the declaration opens on the
/// unrecorded time when it is one interval.
#[test]
fn recording_one_unrecorded_interval_declares_that_interval() {
    let interval = unrecorded(&day(DemoScenario::ProblemPeriods))[0];
    assert_eq!(declared_slots(vec![interval]), [vec![interval]]);
}

/// Several intervals are all offered, where the .NET app left the times to the user.
#[test]
fn recording_several_unrecorded_intervals_offers_each_of_them() {
    let intervals = unrecorded(&day(DemoScenario::ProblemPeriods));
    assert!(
        intervals.len() > 1,
        "the problem periods leave several intervals"
    );
    assert_eq!(declared_slots(intervals.clone()), [intervals]);
}
