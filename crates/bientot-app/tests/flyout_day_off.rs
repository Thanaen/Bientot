//! A day off leaves out the flyout's pause toggle, « Corriger… » and « Fin de journée », which
//! `MainWindow.UpdateCommands` of the .NET app disabled: Tab skips them. It needs the
//! headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

use std::sync::Arc;

use bientot_app::assets::{self, Assets};
use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, Flyout};
use bientot_app::live::Tracking;
use bientot_app::strings::Language;
use bientot_app::theme::ThemeMode;
use bientot_app::theme::size::FLYOUT_WIDTH;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::TrackingState;
use gpui::{
    AppContext as _, FocusHandle, HeadlessAppContext, KeyDownEvent, KeyUpEvent, Keystroke,
    PlatformInput, WindowHandle, px, size,
};

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

/// How many places Tab stops at in the flyout of the same day while tracking is `tracking`.
fn tab_stops(tracking: Tracking) -> usize {
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_asset_source(text_system, Arc::new(Assets));
    cx.update(|cx| {
        cx.set_global(ThemeMode::Light);
        cx.set_global(Language::French);
        assets::load_fonts(cx).unwrap();
        flyout::bind_keys(cx);
    });
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory)
        .unwrap()
        .refresh(Language::French);
    std::fs::remove_dir_all(&directory).ok();
    let day = day.unwrap();
    let window = cx
        .open_window(size(FLYOUT_WIDTH, px(800.)), |window, cx| {
            cx.new(|cx| Flyout::new(day, tracking, None, window, cx))
        })
        .unwrap();
    press(&mut cx, window, "tab");
    let first = focused(&mut cx, window).expect("Tab focuses something");
    for stops in 1..64 {
        press(&mut cx, window, "tab");
        if focused(&mut cx, window).as_ref() == Some(&first) {
            return stops;
        }
    }
    panic!("Tab comes back to where it started");
}

#[test]
fn a_day_off_leaves_the_three_commands_out_of_the_tab_order() {
    let running = |working_day| tab_stops(Tracking::new(TrackingState::Running, working_day));
    assert_eq!((running(true), running(false)), (5, 2));
}
