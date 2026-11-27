//! Where a Linux desktop leaves the app's windows undecorated, as GNOME does on Wayland, window
//! mode's window has the app's own title bar above the flyout, whose « Fermer » quits. Where the
//! desktop decorates it, as KDE Plasma does, the window is the flyout alone. It needs the headless
//! GPUI of the `screenshots` feature, whose platform reports the desktop's decorations, so the
//! app's are set as the screenshots set them.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::cell::Cell;
use std::rc::Rc;

use bientot_app::chrome::{Chrome, ChromeOverride};
use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, Flyout, Quit};
use bientot_app::host::{self, Source, Today};
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_app::theme::size::{TITLEBAR_HEIGHT, WINDOW_CONTROL_WIDTH};
use bientot_core::demo::DemoScenario;
use bientot_platform::placement::{FlyoutPlacement, Frame, InMemoryDesktop};
use bientot_platform::tray::TrayHost;
use gpui::{HeadlessAppContext, Pixels, Size, Tiling, WindowHandle, point};

/// The app in window mode, decorated by `chrome`, with its window open. Quitting counts in
/// `quits` rather than ending the run.
fn window_mode(
    chrome: Chrome,
    quits: &Rc<Cell<usize>>,
) -> (HeadlessAppContext, WindowHandle<Flyout>) {
    let mut cx = common::app(true, flyout::bind_keys);
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let today = Today::open(Source::Demo(Box::new(host)), Language::French).unwrap();
    cx.update(|cx| {
        cx.set_global(ChromeOverride(chrome));
        host::install(today, cx);
        let desktop = InMemoryDesktop {
            placement: FlyoutPlacement::Window,
            usable_height: None,
            frame: Frame::App,
        };
        shell::install(Rc::new(desktop), TrayHost::Absent, cx);
        shell::open_flyout(cx);
        // Registered after the host's: the last listener of an action is the one that runs.
        let quits = quits.clone();
        cx.on_action(move |_: &Quit, _| quits.set(quits.get() + 1));
    });
    cx.run_until_parked();
    let window = cx.update(|cx| cx.windows())[0]
        .downcast::<Flyout>()
        .unwrap();
    common::redraw(&mut cx, window);
    (cx, window)
}

fn size(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> Size<Pixels> {
    cx.update_window(window.into(), |_, window, _| window.bounds().size)
        .unwrap()
}

const UNDECORATED: Chrome = Chrome::App {
    tiling: Tiling {
        top: false,
        left: false,
        right: false,
        bottom: false,
    },
};

#[test]
fn an_undecorated_window_grows_by_the_title_bar_the_app_draws_above_the_flyout() {
    let quits = Rc::default();
    let (mut cx, window) = window_mode(Chrome::Desktop, &quits);
    let decorated = size(&mut cx, window);
    let (mut cx, window) = window_mode(UNDECORATED, &quits);
    let undecorated = size(&mut cx, window);
    assert_eq!(undecorated.width, decorated.width);
    assert_eq!(undecorated.height, decorated.height + TITLEBAR_HEIGHT);
}

#[test]
fn the_title_bars_close_button_quits() {
    let quits = Rc::default();
    let (mut cx, window) = window_mode(UNDECORATED, &quits);
    let width = size(&mut cx, window).width;
    let close = point(width - WINDOW_CONTROL_WIDTH / 2., TITLEBAR_HEIGHT / 2.);
    common::click(&mut cx, window, close);
    assert_eq!(quits.get(), 1, "« Fermer » quits");
}
