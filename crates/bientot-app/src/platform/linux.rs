//! The Linux backend: the tray icon is a StatusNotifierItem of `bientot-linux`, the flyout opens
//! where `bientot-linux` places it on this session, the idle time comes from the first backend of
//! the session that answers, lock, unlock, sleep and resume from logind, the reminders are desktop
//! notifications, an XDG autostart entry starts the app at sign-in, the AppImage adds itself to
//! the menu of applications, a lock and a socket keep one instance per data folder, the
//! environment names the language, and Velopack updates the AppImage. A Flatpak's sandbox shows
//! the reminders through the notification portal and serves the tray icon without a bus name of
//! its own. The other ports answer « unknown » or do nothing, as on a system without a backend,
//! until their lots wire them.

#[cfg(feature = "updates")]
#[path = "direct.rs"]
mod direct;
#[path = "null.rs"]
mod null;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use bientot_core::diagnostics::DiagnosticLogLevel;
use bientot_domain::time::SystemClock;
use bientot_linux::autostart::{Autostart, XdgAutostart};
use bientot_linux::idle::{Backend, IdleNote, IdleProbe};
use bientot_linux::launcher::LauncherEntry;
use bientot_linux::logind::LogindSource;
use bientot_linux::notifications::{DesktopNotifications, Service};
use bientot_linux::placement::LinuxDesktop;
use bientot_linux::tray::StatusNotifier;
use bientot_platform::activity::LastInputProbe;
use bientot_platform::launcher::LauncherPort;
use bientot_platform::lifecycle::LifecycleSource;
use bientot_platform::notifications::{Notifier, QueuedClicks};
use bientot_platform::startup::StartupPort;
use bientot_platform::system::{InstallKind, SystemInfo as _};
use futures::channel::mpsc;
use gpui::{Bounds, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{BackendNote, Backends, Platform, PlatformContext};
use crate::zones::SystemZones;

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
    let demo = ctx.launch.demo.is_some();
    // A demo neither reads the input nor watches the session: as on Windows, its day is the
    // scenario's. The probe opens its backend now, so a demo does not make one.
    let (notes, backend_notes) = mpsc::unbounded();
    let idle = (!demo).then(|| {
        IdleProbe::of_this_session(move |note| {
            notes.unbounded_send(idle_line(note)).ok();
        })
    });
    let lifecycle = (!demo).then(|| LogindSource::new(SystemClock, SystemZones));
    // A demo shows no notifications, as on Windows.
    let clicks = QueuedClicks::default();
    let kind = this_install_kind();
    let service = Service::for_install(kind);
    let notifier = ctx.launch.demo.is_none().then(|| {
        DesktopNotifications::start(service, clicks.clone())
            .map(|notifications| Box::new(notifications) as Box<dyn Notifier>)
    });
    let (events, tray_events) = mpsc::unbounded();
    let tray = StatusNotifier::new(kind, move |event| {
        events.unbounded_send(event).ok();
    });
    let desktop = Rc::new(LinuxDesktop::of_this_session());
    DESKTOP.with_borrow_mut(|shared| *shared = Some(Rc::clone(&desktop)));
    // A demo leaves the user's registration alone, as on Windows. The Background portal's
    // backend is T112's: until then a Flatpak gets no switch rather than an entry in the
    // sandbox's own autostart folder, which no desktop reads.
    let startup = match (ctx.launch.demo, Autostart::for_install(kind)) {
        (None, Autostart::Entry) => {
            Some(Arc::new(XdgAutostart::for_this_process()) as Arc<dyn StartupPort>)
        }
        (Some(_), _) | (None, Autostart::BackgroundPortal) => None,
    };
    // The AppImage alone writes its own entry in the menu; a demo leaves the menu alone.
    let launcher = ctx
        .launch
        .demo
        .is_none()
        .then(LauncherEntry::for_this_process)
        .flatten()
        .map(|launcher| Arc::new(launcher) as Arc<dyn LauncherPort>);
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
        idle: Some(backend_name(idle.as_ref().and_then(IdleProbe::backend))),
        lifecycle: Some(if lifecycle.is_some() {
            "logind"
        } else {
            "none"
        }),
    };
    let null = null::build(ctx)?;
    Ok(Platform {
        last_input: match idle {
            Some(probe) => Box::new(probe) as Box<dyn LastInputProbe + Send>,
            None => null.last_input,
        },
        lifecycle: match lifecycle {
            Some(source) => Arc::new(source) as Arc<dyn LifecycleSource + Send + Sync>,
            None => null.lifecycle,
        },
        notifier,
        startup,
        launcher,
        notification_clicks: Box::new(clicks),
        backends,
        backend_notes,
        tray: Rc::new(tray),
        tray_events,
        desktop,
        ..null
    })
}

/// The diagnostic log's line for what the idle probe noted (contracts/linux-backends.md,
/// « Idle »). Without a backend the built-in source cannot count activity: a warning.
fn idle_line(note: IdleNote) -> BackendNote {
    let level = |backend: Option<Backend>| match backend {
        Some(_) => DiagnosticLogLevel::Information,
        None => DiagnosticLogLevel::Warning,
    };
    let name = |backend| backend_name(backend).into();
    match note {
        IdleNote::Selected(backend) => BackendNote {
            level: level(backend),
            event: "idle.backend",
            properties: vec![("name", name(backend))],
        },
        IdleNote::Changed { from, to } => BackendNote {
            level: level(to),
            event: "idle.backend_changed",
            properties: vec![("from", name(from)), ("to", name(to))],
        },
        IdleNote::InhibitorsCount => BackendNote {
            level: DiagnosticLogLevel::Information,
            event: "idle.inhibitor_blind",
            properties: vec![("value", false.into())],
        },
    }
}

fn backend_name(backend: Option<Backend>) -> &'static str {
    backend.map_or("none", Backend::name)
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

#[cfg(test)]
mod tests {
    use bientot_core::diagnostics::DiagnosticLogLevel::{Information, Warning};
    use bientot_linux::idle::{Backend, IdleNote};

    use super::{BackendNote, idle_line};

    #[test]
    fn the_log_names_the_idle_backend_chosen_then_each_change() {
        let line = |level, event, properties: &[(&'static str, &'static str)]| BackendNote {
            level,
            event,
            properties: properties
                .iter()
                .map(|&(name, value)| (name, value.into()))
                .collect(),
        };
        assert_eq!(
            [
                IdleNote::Selected(Some(Backend::XScreenSaver)),
                IdleNote::Changed {
                    from: Some(Backend::XScreenSaver),
                    to: None,
                },
                IdleNote::Selected(None),
                IdleNote::Changed {
                    from: None,
                    to: Some(Backend::MutterIdleMonitor),
                },
            ]
            .map(idle_line),
            [
                line(Information, "idle.backend", &[("name", "MIT-SCREEN-SAVER")]),
                line(
                    Warning,
                    "idle.backend_changed",
                    &[("from", "MIT-SCREEN-SAVER"), ("to", "none")],
                ),
                line(Warning, "idle.backend", &[("name", "none")]),
                line(
                    Information,
                    "idle.backend_changed",
                    &[("from", "none"), ("to", "org.gnome.Mutter.IdleMonitor")],
                ),
            ]
        );
    }

    #[test]
    fn the_log_says_when_an_app_that_inhibits_idle_counts_as_activity() {
        assert_eq!(
            idle_line(IdleNote::InhibitorsCount),
            BackendNote {
                level: Information,
                event: "idle.inhibitor_blind",
                properties: vec![("value", false.into())],
            }
        );
    }

    #[cfg(feature = "updates")]
    #[test]
    fn velopack_updates_the_appimage_alone() {
        use bientot_platform::system::InstallKind;

        use super::packed_by_velopack;

        let updated: Vec<_> = InstallKind::ALL
            .into_iter()
            .filter(|kind| packed_by_velopack(*kind))
            .collect();
        assert_eq!(updated, [InstallKind::AppImage]);
    }
}
