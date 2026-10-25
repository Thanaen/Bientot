//! The unsaved-changes dialog of « Paramètres » comes in over `motion::DIALOG`, takes keys from
//! its first frame and then leaves the window idle. It needs the headless GPUI of the
//! `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use bientot_app::settings::{Field, SettingsForm, UsagePingFacts};
use bientot_app::settings_view::{self, SettingsView};
use bientot_app::strings::Language;
use bientot_app::theme::motion;
use bientot_core::settings::EditGroup;
use bientot_domain::configuration::Settings;
use bientot_domain::usage_ping::{Build, Channel};
use gpui::{HeadlessAppContext, KeyDownEvent, KeyUpEvent, Keystroke, PlatformInput, WindowHandle};

fn open(reduce_motion: bool) -> (HeadlessAppContext, WindowHandle<SettingsView>) {
    let mut cx = common::app(reduce_motion, settings_view::bind_keys);
    let usage_ping = UsagePingFacts {
        build: Build::new("1.0.0", Channel::Store).unwrap(),
        released: None,
        today: jiff::civil::date(2026, 10, 13),
        last_sent: None,
    };
    let form = SettingsForm::new(Settings::default(), usage_ping, Language::French);
    let window = common::open(
        &mut cx,
        gpui::size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT),
        |window, cx| SettingsView::new(form, None, window, cx),
    );
    (cx, window)
}

/// An invalid daily target, then a request to close: the dialog shows instead.
fn ask_to_close(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>) {
    cx.update(|cx| {
        window
            .update(cx, |view, window, cx| {
                view.type_text(Field::Target, "25:00", cx);
                view.form_mut().commit(EditGroup::Targets);
                assert!(!view.should_close(window, cx), "the window closes as is");
            })
            .unwrap();
    });
    cx.run_until_parked();
}

fn dialog_shows(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>) -> bool {
    cx.update(|cx| window.read(cx).unwrap().form().dialog().is_some())
}

fn press(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>, key: &str) {
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
    })
    .unwrap();
    cx.run_until_parked();
}

fn next_frame(cx: &mut HeadlessAppContext, window: WindowHandle<SettingsView>) -> usize {
    common::next_frame(cx, window.into())
}

#[test]
fn the_dialog_takes_keys_from_its_first_frame() {
    let (mut cx, window) = open(false);
    ask_to_close(&mut cx, window);
    assert!(dialog_shows(&mut cx, window));
    press(&mut cx, window, "escape");
    assert!(
        !dialog_shows(&mut cx, window),
        "Échap keeps editing while the dialog comes in"
    );

    ask_to_close(&mut cx, window);
    press(&mut cx, window, "tab");
    press(&mut cx, window, "enter");
    assert!(
        cx.update_window(window.into(), |_, _, _| ()).is_err(),
        "« Ignorer les changements » closes the window while the dialog comes in"
    );
}

#[test]
fn the_dialog_comes_to_rest_and_comes_in_again_on_each_opening() {
    let (mut cx, window) = open(false);
    next_frame(&mut cx, window);
    assert_eq!(next_frame(&mut cx, window), 0, "the window is busy before");

    for opening in ["first", "second"] {
        ask_to_close(&mut cx, window);
        common::assert_entrance_runs(
            &mut cx,
            window.into(),
            motion::DIALOG,
            &format!("the dialog on its {opening} opening"),
        );
        assert!(dialog_shows(&mut cx, window));
        press(&mut cx, window, "escape");
    }
}

#[test]
fn with_reduced_motion_the_dialog_shows_at_rest() {
    let (mut cx, window) = open(true);
    ask_to_close(&mut cx, window);
    assert!(dialog_shows(&mut cx, window));
    assert_eq!(next_frame(&mut cx, window), 0);
}

#[test]
fn turning_reduced_motion_off_does_not_replay_a_dialog_at_rest() {
    let (mut cx, window) = open(true);
    ask_to_close(&mut cx, window);
    next_frame(&mut cx, window);
    cx.update(|cx| cx.set_reduce_motion(false));
    cx.update_window(window.into(), |_, window, _| window.refresh())
        .unwrap();
    next_frame(&mut cx, window);
    assert_eq!(next_frame(&mut cx, window), 0, "the entrance plays again");
}
