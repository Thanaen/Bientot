//! In window mode (FR-012) the flyout says once why Bientôt shows in a window: « Comment afficher
//! l’icône » opens the help page, « Compris » closes the notice for good, across restarts. With a
//! tray host, as on Windows, whose notification area is always there, it never shows. A keyboard
//! user meets the notice as two more Tab stops, right after the header's two buttons when no
//! callout about the day's time comes first. It needs the headless GPUI of the `screenshots`
//! feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, DismissWindowModeNotice, Flyout, ShowIconHelp};
use bientot_app::host::{self, Source, Today};
use bientot_app::live::{self, LiveHost};
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_core::demo::DemoScenario;
use bientot_domain::sources::LastInputReading;
use bientot_domain::time::{FixedTimeZoneProvider, SystemClock};
use bientot_platform::activity::LastInputProbe;
use bientot_platform::placement::{FlyoutPlacement, InMemoryDesktop};
use bientot_platform::tray::TrayHost;
use bientot_store::data_directory::DataDirectory;
use gpui::{HeadlessAppContext, WindowHandle};
use jiff::tz::TimeZone;

/// The header's « Ouvrir la semaine » and « Paramètres », before the notice's buttons.
const HEADER_STOPS: usize = 2;

struct NoInput;

impl LastInputProbe for NoInput {
    fn read(&self) -> Option<LastInputReading> {
        None
    }
}

/// The app showing `today`, on a desktop that opens the flyout in a window without a tray host, as
/// `LinuxDesktop` does; « Comment afficher l’icône » counts its presses in `help` rather than
/// opening a browser.
fn started(today: Today, tray: TrayHost, help: &Rc<Cell<usize>>) -> HeadlessAppContext {
    let mut cx = common::app(true, flyout::bind_keys);
    let help = help.clone();
    cx.update(|cx| {
        host::install(today, cx);
        let desktop = Rc::new(InMemoryDesktop {
            placement: match tray {
                TrayHost::Present => FlyoutPlacement::CompositorPlaced,
                TrayHost::Absent => FlyoutPlacement::Window,
            },
            usable_height: None,
            system_draws_frame: false,
        });
        shell::install(desktop, tray, cx);
        // Registered after the host's: the last listener of an action is the one that runs.
        cx.on_action(move |_: &ShowIconHelp, _| help.set(help.get() + 1));
    });
    cx
}

fn demo_day() -> Today {
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    Today::open(Source::Demo(Box::new(host)), Language::French).unwrap()
}

/// The live day of the data in `folder`, as a launch opens it.
fn live_day(folder: &Path) -> Today {
    let directory = DataDirectory::from(folder.to_owned());
    let connection = bientot_store::database::open(&directory.database_file()).unwrap();
    let host = LiveHost::start(
        &directory,
        live::load_settings(&directory).unwrap(),
        Box::leak(Box::new(connection)),
        &SystemClock,
        Box::leak(Box::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC))),
        &NoInput,
    )
    .unwrap();
    Today::open(Source::Live(Box::new(host)), Language::French).unwrap()
}

/// The flyout as the app shows it, or its window in window mode.
fn opened(cx: &mut HeadlessAppContext) -> WindowHandle<Flyout> {
    cx.update(shell::open_flyout);
    cx.run_until_parked();
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), 1);
    let window = windows[0].downcast::<Flyout>().unwrap();
    common::redraw(cx, window);
    window
}

/// How many controls Tab reaches before it comes back to the first, which keeps the focus.
fn tab_stops(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> usize {
    let focused = |cx: &mut HeadlessAppContext| {
        cx.update_window(window.into(), |_, window, cx| window.focused(cx))
            .unwrap()
    };
    common::press(cx, window, "tab");
    let first = focused(cx).expect("Tab focuses something");
    let mut stops = 1;
    loop {
        common::press(cx, window, "tab");
        if focused(cx).as_ref() == Some(&first) {
            return stops;
        }
        stops += 1;
        assert!(stops < 32, "Tab comes back to where it started");
    }
}

/// Enter on the `stop`-th Tab stop, counted from 1, with the focus on the first.
fn press_stop(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, stop: usize) {
    for _ in 1..stop {
        common::press(cx, window, "tab");
    }
    common::press(cx, window, "enter");
}

#[test]
fn without_a_tray_host_the_flyout_says_why_it_shows_in_a_window() {
    let help = Rc::default();
    let mut cx = started(demo_day(), TrayHost::Present, &help);
    let flyout = opened(&mut cx);
    let plain = tab_stops(&mut cx, flyout);

    let mut cx = started(demo_day(), TrayHost::Absent, &help);
    let window = opened(&mut cx);
    assert_eq!(tab_stops(&mut cx, window), plain + 2);
    press_stop(&mut cx, window, HEADER_STOPS + 1);
    assert_eq!(
        help.get(),
        1,
        "« Comment afficher l’icône » opens the help page"
    );
}

#[test]
fn a_tray_host_never_shows_the_notice_even_after_window_mode() {
    let help = Rc::default();
    let mut cx = started(demo_day(), TrayHost::Absent, &help);
    let window = opened(&mut cx);
    let with_notice = tab_stops(&mut cx, window);

    // The window the user is in becomes the flyout once a host appears.
    cx.update(|cx| shell::tray_changed(TrayHost::Present, cx));
    cx.run_until_parked();
    let flyout = opened(&mut cx);
    assert_eq!(tab_stops(&mut cx, flyout), with_notice - 2);
    press_stop(&mut cx, flyout, HEADER_STOPS + 1);
    assert_eq!(help.get(), 0, "the third stop is no longer the help page");
}

#[test]
fn got_it_closes_the_notice_and_it_stays_closed_after_a_restart() {
    let folder = tempfile::tempdir().unwrap();
    let help = Rc::default();
    let mut cx = started(live_day(folder.path()), TrayHost::Absent, &help);
    let window = opened(&mut cx);
    let with_notice = tab_stops(&mut cx, window);

    // The live day opens with callouts about its time above the notice, so « Compris » is not
    // at a fixed Tab stop here; the demo tests reach the notice by keyboard.
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(DismissWindowModeNotice), cx);
    })
    .unwrap();
    common::redraw(&mut cx, window);
    assert_eq!(
        cx.update(|cx| cx.windows()),
        [window.into()],
        "« Compris » keeps the window"
    );
    assert_eq!(tab_stops(&mut cx, window), with_notice - 2);
    drop(cx);

    let mut cx = started(live_day(folder.path()), TrayHost::Absent, &help);
    let window = opened(&mut cx);
    assert_eq!(tab_stops(&mut cx, window), with_notice - 2);
    assert_eq!(help.get(), 0);
}
