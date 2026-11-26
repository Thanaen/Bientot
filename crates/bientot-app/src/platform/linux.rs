//! The Linux backend: the tray icon is a StatusNotifierItem of `bientot-linux`, the flyout opens
//! where `bientot-linux` places it on this session, and the reminders are desktop notifications.
//! The other ports answer « unknown » or do nothing, as on a system without a backend, until
//! their lots wire them.

#[path = "null.rs"]
mod null;

use std::cell::RefCell;
use std::rc::Rc;

use bientot_linux::notifications::{DesktopNotifications, Service};
use bientot_linux::placement::LinuxDesktop;
use bientot_linux::tray::StatusNotifier;
use bientot_platform::notifications::{Notifier, QueuedClicks};
use futures::channel::mpsc;
use gpui::{Bounds, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{Platform, PlatformContext};

// `build`, `place_window`, `activate_window` and `NotificationsRefused` below take the place of
// null's.
pub use null::*;

/// No notification server, or no portal in a Flatpak, answered at start: the app shows no
/// reminder, so that none is reserved in the ledger without being shown, and the flyout says so.
pub type NotificationsRefused = bientot_linux::notifications::Unavailable;

thread_local! {
    /// The desktop `build` made, which moves and focuses the flyout's window on X11.
    static DESKTOP: RefCell<Option<Rc<LinuxDesktop>>> = const { RefCell::new(None) };
}

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform> {
    // A demo shows no notifications, as on Windows.
    let clicks = QueuedClicks::default();
    let notifier = ctx.launch.demo.is_none().then(|| {
        DesktopNotifications::start(Service::of_this_process(), clicks.clone())
            .map(|notifications| Box::new(notifications) as Box<dyn Notifier>)
    });
    let (events, tray_events) = mpsc::unbounded();
    let tray = StatusNotifier::new(move |event| {
        events.unbounded_send(event).ok();
    });
    let desktop = Rc::new(LinuxDesktop::of_this_session());
    DESKTOP.with_borrow_mut(|shared| *shared = Some(Rc::clone(&desktop)));
    Ok(Platform {
        notifier,
        notification_clicks: Box::new(clicks),
        tray_backend: Some(tray.backend()),
        tray: Rc::new(tray),
        tray_events,
        desktop,
        ..null::build(ctx)?
    })
}

/// Resizes `window` to `client`, and on X11 moves it there: elsewhere the compositor places it.
pub fn place_window(window: &mut Window, client: Bounds<i32>) {
    null::place_window(window, client);
    with_x11_window(window, |x11, id| {
        x11.move_window(id, client.origin.x, client.origin.y)
    });
}

pub fn activate_window(window: &mut Window) {
    null::activate_window(window);
    with_x11_window(window, |x11, id| x11.focus(id));
}

/// Runs `act` on the X server and the X11 id of `window`, when the session is X11.
fn with_x11_window(
    window: &Window,
    act: impl FnOnce(
        &bientot_linux::placement::X11,
        u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>,
) {
    let id = match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Xcb(handle)) => handle.window.get(),
        Ok(RawWindowHandle::Xlib(handle)) => handle.window as u32,
        _ => return,
    };
    DESKTOP.with_borrow(|desktop| {
        if let Some(x11) = desktop.as_deref().and_then(LinuxDesktop::x11)
            && let Err(error) = act(x11, id)
        {
            log::warn!("cannot place the flyout's X11 window: {error}");
        }
    });
}
