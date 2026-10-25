//! What a button of the flyout changes shows in the flyout at once, not at the next periodic
//! refresh: the action reaches the day while the flyout's window is being updated. It needs the
//! headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, Flyout};
use bientot_app::host::{self, Source, Today};
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::TrackingState;

#[test]
fn a_pause_pressed_in_the_flyout_shows_in_it() {
    let mut cx = common::app(true, flyout::bind_keys);
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let today = Today::open(Source::Demo(Box::new(host)), Language::French).unwrap();
    cx.update(|cx| {
        host::install(today, cx);
        shell::open_flyout(cx);
    });
    cx.run_until_parked();
    let window = cx.update(|cx| cx.windows()).into_iter().next().unwrap();
    let window = window.downcast::<Flyout>().unwrap();
    let shown = |cx: &mut gpui::HeadlessAppContext| {
        cx.update(|cx| window.read(cx).unwrap().shown_tracking())
    };
    assert_eq!(shown(&mut cx), TrackingState::Running);
    common::press(&mut cx, window, "ctrl-alt-p");
    assert_eq!(shown(&mut cx), TrackingState::Paused);
}
