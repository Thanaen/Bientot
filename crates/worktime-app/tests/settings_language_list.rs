//! The language list of « Paramètres » comes in under its button over `motion::MENU` each time it
//! opens, then leaves the window idle. It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use gpui::{HeadlessAppContext, WindowHandle};
use worktime_app::settings::{SettingsForm, UsagePingFacts};
use worktime_app::settings_view::{self, SettingsView};
use worktime_app::strings::Language;
use worktime_app::theme::motion;
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::usage_ping::{Build, Channel};

fn open(reduce_motion: bool) -> (HeadlessAppContext, WindowHandle<SettingsView>) {
    let mut cx = common::app(reduce_motion, settings_view::bind_keys);
    let usage_ping = UsagePingFacts {
        build: Build::new("1.0.0", Channel::Store).unwrap(),
        released: None,
        today: jiff::civil::date(2026, 10, 13),
        last_sent: None,
    };
    let form = SettingsForm::new(WorkTimeSettings::default(), usage_ping, Language::French);
    let window = common::open(
        &mut cx,
        gpui::size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT),
        |window, cx| SettingsView::new(form, None, window, cx),
    );
    next_frame(&mut cx, window);
    (cx, window)
}

fn toggle(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>) {
    cx.update(|cx| {
        window
            .update(cx, |view, _, cx| view.toggle_language_list(cx))
            .unwrap();
    });
    cx.run_until_parked();
}

fn next_frame(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>) -> usize {
    common::next_frame(cx, window.into())
}

#[test]
fn the_language_list_comes_in_on_each_opening_then_rests() {
    let (mut cx, window) = open(false);
    assert_eq!(next_frame(&mut cx, window), 0, "the window is busy before");
    for opening in ["first", "second"] {
        toggle(&mut cx, window);
        common::assert_entrance_runs(
            &mut cx,
            window.into(),
            motion::MENU,
            &format!("the list on its {opening} opening"),
        );
        toggle(&mut cx, window);
        assert_eq!(next_frame(&mut cx, window), 0, "the list closes at once");
    }
}

#[test]
fn with_reduced_motion_the_language_list_shows_at_rest() {
    let (mut cx, window) = open(true);
    toggle(&mut cx, window);
    assert_eq!(next_frame(&mut cx, window), 0);
    toggle(&mut cx, window);

    cx.update(|cx| cx.set_reduce_motion(false));
    toggle(&mut cx, window);
    assert!(
        next_frame(&mut cx, window) > 0,
        "no entrance once reduced motion is off"
    );
}
