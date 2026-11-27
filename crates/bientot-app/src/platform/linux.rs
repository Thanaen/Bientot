//! The Linux backend: the tray icon is a StatusNotifierItem of `bientot-linux`, the flyout opens
//! where `bientot-linux` places it on this session, the reminders are desktop notifications, an
//! XDG autostart entry starts the app at sign-in, a lock and a socket keep one instance per data
//! folder, the environment names the language, and Velopack updates the AppImage. The other ports
//! answer « unknown » or do nothing, as on a system without a backend, until their lots wire them.

#[cfg(feature = "updates")]
#[path = "direct.rs"]
mod direct;
#[path = "null.rs"]
mod null;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use bientot_linux::autostart::XdgAutostart;
use bientot_linux::notifications::{DesktopNotifications, Service};
use bientot_linux::placement::LinuxDesktop;
use bientot_linux::tray::StatusNotifier;
use bientot_platform::notifications::{Notifier, QueuedClicks};
use bientot_platform::startup::StartupPort;
#[cfg(feature = "updates")]
use bientot_platform::system::{InstallKind, SystemInfo as _};
use futures::channel::mpsc;
use gpui::{Bounds, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{Backends, Platform, PlatformContext};

// `build`, `place_window`, `activate_window`, `NotificationsRefused`, `Instance`,
// `wait_for_exit`, `system_locale` and `system` below take the place of null's, and with `updates`
// `run_update_hooks`, `install_pending_update` and `direct_updates`.
pub use null::*;

pub use bientot_linux::process::wait_for_exit;
pub use bientot_linux::single_instance::PrimaryInstance as Instance;

/// `LC_ALL`, `LC_MESSAGES` or `LANG`, as POSIX reads them.
pub fn system_locale() -> bientot_linux::locale::EnvironmentLocale {
    bientot_linux::locale::EnvironmentLocale
}

/// The install kind and the session from the environment, the distribution from os-release.
pub fn system() -> Option<bientot_linux::system::ThisSystem> {
    Some(bientot_linux::system::ThisSystem)
}

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
    let service = Service::of_this_process();
    let notifier = ctx.launch.demo.is_none().then(|| {
        DesktopNotifications::start(service, clicks.clone())
            .map(|notifications| Box::new(notifications) as Box<dyn Notifier>)
    });
    let (events, tray_events) = mpsc::unbounded();
    let tray = StatusNotifier::new(move |event| {
        events.unbounded_send(event).ok();
    });
    let desktop = Rc::new(LinuxDesktop::of_this_session());
    DESKTOP.with_borrow_mut(|shared| *shared = Some(Rc::clone(&desktop)));
    // A demo leaves the user's registration alone, as on Windows.
    let startup = ctx
        .launch
        .demo
        .is_none()
        .then(|| Arc::new(XdgAutostart::for_this_process()) as Arc<dyn StartupPort>);
    let backends = Backends {
        tray: Some(tray.backend()),
        notifications: Some(match &notifier {
            Some(Ok(_)) => service.backend(),
            Some(Err(_)) | None => "none",
        }),
        autostart: Some(if startup.is_some() {
            "xdg-autostart"
        } else {
            "none"
        }),
    };
    Ok(Platform {
        notifier,
        startup,
        notification_clicks: Box::new(clicks),
        backends,
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

/// Whether Velopack packed this copy, the only kind it updates: the AppImage. A Flatpak updates
/// from its remote, and a development build is no package to replace.
#[cfg(feature = "updates")]
fn packed_by_velopack(kind: InstallKind) -> bool {
    kind == InstallKind::AppImage
}

#[cfg(feature = "updates")]
fn this_install_kind() -> InstallKind {
    bientot_linux::system::ThisSystem.install_kind()
}

/// Velopack's hooks, run before anything else, in the AppImage only.
#[cfg(feature = "updates")]
pub fn run_update_hooks() -> bool {
    packed_by_velopack(this_install_kind()) && direct::run_hooks(velopack::VelopackApp::build())
}

#[cfg(feature = "updates")]
pub fn install_pending_update() {
    if packed_by_velopack(this_install_kind()) {
        direct::install_pending();
    }
}

/// The feed of the AppImage. `None` for any other install, and when Velopack finds no package
/// around the running executable.
#[cfg(feature = "updates")]
pub fn direct_updates() -> Option<direct::DirectUpdates> {
    packed_by_velopack(this_install_kind())
        .then(direct::DirectUpdates::installed)
        .flatten()
}

/// Velopack's errors carry no HRESULT off Windows.
#[cfg(feature = "updates")]
impl crate::updates::StoreError for velopack::Error {
    fn hresult(&self) -> Option<i32> {
        None
    }
}

#[cfg(all(test, feature = "updates"))]
mod tests {
    use bientot_platform::system::InstallKind;

    use super::packed_by_velopack;

    #[test]
    fn velopack_updates_the_appimage_alone() {
        let updated: Vec<_> = [
            InstallKind::Store,
            InstallKind::Direct,
            InstallKind::Portable,
            InstallKind::Flatpak,
            InstallKind::AppImage,
            InstallKind::Development,
        ]
        .into_iter()
        .filter(|kind| packed_by_velopack(*kind))
        .collect();
        assert_eq!(updated, [InstallKind::AppImage]);
    }
}
