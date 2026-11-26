//! Window mode (FR-012): without a tray host the flyout is an ordinary window, which the host's
//! return turns back into the flyout. It needs the headless GPUI of the `screenshots` feature,
//! whose windows keep the focus once activated and cannot be minimized, so a window in the
//! background is left to `shell`'s unit tests.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::rc::Rc;

use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout;
use bientot_app::host::{self, Source, Today};
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_core::demo::DemoScenario;
use bientot_platform::placement::{Desktop, FlyoutPlacement};
use bientot_platform::tray::{ScreenPoint, TrayHost};
use gpui::{AnyWindowHandle, HeadlessAppContext};

/// A desktop that places the flyout where the compositor puts it, or in a window without a host,
/// as `LinuxDesktop` does.
struct Compositor;

impl Desktop for Compositor {
    fn flyout_placement(&self, _click: Option<ScreenPoint>, tray: TrayHost) -> FlyoutPlacement {
        match tray {
            TrayHost::Present => FlyoutPlacement::CompositorPlaced,
            TrayHost::Absent => FlyoutPlacement::Window,
        }
    }

    fn usable_height(&self, _click: Option<ScreenPoint>) -> Option<i32> {
        None
    }

    fn system_draws_frame(&self) -> bool {
        false
    }
}

fn started(tray: TrayHost) -> HeadlessAppContext {
    let mut cx = common::app(true, flyout::bind_keys);
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let today = Today::open(Source::Demo(Box::new(host)), Language::French).unwrap();
    cx.update(|cx| {
        host::install(today, cx);
        shell::install(Rc::new(Compositor), tray, cx);
    });
    cx
}

fn windows(cx: &mut HeadlessAppContext) -> Vec<AnyWindowHandle> {
    cx.run_until_parked();
    cx.update(|cx| cx.windows())
}

#[test]
fn a_host_that_leaves_while_nothing_shows_brings_the_app_back_in_its_window() {
    let mut cx = started(TrayHost::Present);
    assert!(windows(&mut cx).is_empty());

    cx.update(|cx| shell::tray_changed(TrayHost::Absent, cx));
    let window = windows(&mut cx);
    assert_eq!(window.len(), 1, "nothing else would show the app");
    assert!(cx.update(|cx| shell::window_mode(cx)));

    // A second launch while the window shows brings that window to the front.
    cx.update(shell::open_flyout);
    assert_eq!(windows(&mut cx), window);
}

#[test]
fn the_flyout_becomes_a_window_when_the_host_leaves_and_the_flyout_again_when_it_returns() {
    let mut cx = started(TrayHost::Present);
    cx.update(shell::open_flyout);
    let flyout = windows(&mut cx);
    assert_eq!(flyout.len(), 1);
    assert!(!cx.update(|cx| shell::window_mode(cx)));

    cx.update(|cx| shell::tray_changed(TrayHost::Absent, cx));
    let window = windows(&mut cx);
    assert_eq!(window.len(), 1);
    assert_ne!(window[0].window_id(), flyout[0].window_id());
    assert!(cx.update(|cx| shell::window_mode(cx)));

    // The window has the focus, as the headless windows do: the user is looking at it.
    cx.update(|cx| shell::tray_changed(TrayHost::Present, cx));
    let flyout = windows(&mut cx);
    assert_eq!(flyout.len(), 1);
    assert_ne!(flyout[0].window_id(), window[0].window_id());
    assert!(!cx.update(|cx| shell::window_mode(cx)));
}
