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
use std::io;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context as _;
use bientot_core::diagnostics::DiagnosticLogLevel;
use bientot_core::notifications::NotificationError;
use bientot_domain::time::SystemClock;
use bientot_linux::autostart::{Autostart, XdgAutostart};
use bientot_linux::idle::{Backend, IdleNote, IdleWatch};
use bientot_linux::launcher::LauncherEntry;
use bientot_linux::logind::LogindSource;
use bientot_linux::notifications::{DesktopNotifications, Service, Unavailable};
use bientot_linux::placement::LinuxDesktop;
use bientot_linux::single_instance::{ActivationListener, InstanceNote, PrimaryInstance};
use bientot_linux::tray::StatusNotifier;
use bientot_platform::activity::LastInputProbe;
use bientot_platform::launcher::LauncherPort;
use bientot_platform::lifecycle::{
    LifecycleListener, LifecycleSource, LifecycleTransition, LockReporting, SessionSnapshot,
};
use bientot_platform::notifications::{Notifier, QueuedClicks};
use bientot_platform::single_instance::{Acquired, InstanceLock};
use bientot_platform::startup::StartupPort;
use bientot_platform::system::{InstallKind, SystemInfo as _};
use bientot_store::data_directory::DataDirectory;
use futures::channel::mpsc;
use gpui::{Bounds, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{BackendNote, Backends, NO_BACKEND, Platform, PlatformContext, lock_reporting_name};
use crate::diagnostics::error_kind;
use crate::zones::SystemZones;

// `build`, `place_window`, `activate_window`, `NotificationsRefused`, `Instance`,
// `wait_for_exit`, `show_fatal`, `system_locale` and `system` below take the place of null's, and
// with `updates` `run_update_hooks`, `install_pending_update` and `direct_updates`.
pub use null::*;

pub use bientot_linux::process::wait_for_exit;

/// The lock and the socket of `bientot-linux`. What goes wrong with them reaches the diagnostic
/// log among the notes of the platform `build` made: the folder is claimed before the platform
/// is built, and listens after.
pub struct Instance(PrimaryInstance);

impl InstanceLock for Instance {
    type Listener = ActivationListener;

    fn acquire(directory: &DataDirectory) -> io::Result<Acquired<Self>> {
        Ok(match PrimaryInstance::acquire(directory)? {
            Acquired::Primary(instance) => Acquired::Primary(Self(instance)),
            Acquired::Secondary => Acquired::Secondary,
        })
    }

    /// Without a platform built, as after a failed start, which has no diagnostic log, its notes
    /// go nowhere.
    fn listen(
        self,
        on_activation: impl FnMut() + Send + 'static,
    ) -> io::Result<ActivationListener> {
        let notes = NOTES.lock().unwrap_or_else(PoisonError::into_inner).clone();
        self.0.listen(on_activation, move |note| {
            if let Some(notes) = &notes {
                notes.unbounded_send(instance_line(note)).ok();
            }
        })
    }
}

/// Where the notes of the platform `build` made last go, for the instance claimed before it.
static NOTES: Mutex<Option<mpsc::UnboundedSender<BackendNote>>> = Mutex::new(None);

/// `LC_ALL`, `LC_MESSAGES` or `LANG`, as POSIX reads them.
pub fn system_locale() -> bientot_linux::locale::EnvironmentLocale {
    bientot_linux::locale::EnvironmentLocale
}

/// The install kind and the session from the environment, the distribution from os-release.
pub fn system() -> Option<bientot_linux::system::ThisSystem> {
    Some(bientot_linux::system::ThisSystem)
}

/// The caller already wrote it to the standard error, which a desktop launch shows nowhere: a
/// notification tells it too, when a server answers, or the portal in a Flatpak.
pub fn show_fatal(title: &str, body: &str) {
    let service = Service::for_install(this_install_kind());
    let request = bientot_linux::notifications::last_resort(title, body);
    let _ = bientot_linux::notifications::show_once(service, &request);
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
    let kind = this_install_kind();
    // A demo neither reads the input nor watches the session: as on Windows, its day is the
    // scenario's. The watch's thread opens the idle backend, so a demo starts none.
    let (notes, backend_notes) = mpsc::unbounded();
    *NOTES.lock().unwrap_or_else(PoisonError::into_inner) = Some(notes.clone());
    let refusals = notes.clone();
    let launches = notes.clone();
    let lifecycle = (!demo).then(|| Reported {
        source: LogindSource::new(kind, SystemClock, SystemZones),
        notes: notes.clone(),
    });
    let idle = (!demo)
        .then(|| {
            IdleWatch::of_this_session(move |note| {
                notes.unbounded_send(idle_line(note)).ok();
            })
        })
        .transpose()
        .context("the idle watch's thread did not start")?;
    // A demo shows no notifications, as on Windows.
    let clicks = QueuedClicks::default();
    let service = Service::for_install(kind);
    let notifier = ctx.launch.demo.is_none().then(|| {
        notifications(service, clicks.clone(), refusals)
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
        .map(|entry| {
            Arc::new(ReportedLauncher {
                entry,
                notes: launches,
            }) as Arc<dyn LauncherPort>
        });
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
        // The watch's first note names the backend once the thread has chosen it.
        idle: demo.then_some(NO_BACKEND),
        activity_undetectable: false,
        // Until a note says the watch did not start.
        lifecycle: Some(if lifecycle.is_some() {
            "logind"
        } else {
            "none"
        }),
        // The watch tells once it starts.
        lock_reporting: None,
    };
    let null = null::build(ctx)?;
    Ok(Platform {
        last_input: match idle {
            Some(watch) => Box::new(watch) as Box<dyn LastInputProbe + Send>,
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

/// The notifications through `service`, whose refused shows `notes` gets as
/// [`notification_failed`].
fn notifications(
    service: Service,
    clicks: QueuedClicks,
    notes: mpsc::UnboundedSender<BackendNote>,
) -> Result<DesktopNotifications, Unavailable> {
    DesktopNotifications::start(service, clicks, move || {
        notes.unbounded_send(notification_failed()).ok();
    })
}

/// The diagnostic log's line for a show the notification server or the portal refused, the line
/// of a refused toast on Windows, with the type it names there. Its `message` is left out: the
/// bus's text can name the user's session or bus.
fn notification_failed() -> BackendNote {
    BackendNote {
        level: DiagnosticLogLevel::Error,
        event: "notification.failed",
        properties: vec![(
            "exceptionType",
            std::any::type_name::<NotificationError>().into(),
        )],
        idle: None,
        lifecycle: None,
        lock_reporting: None,
    }
}

/// The diagnostic log's line for what keeps later launches from reaching this Bientôt
/// (contracts/linux-backends.md, « Single instance »).
fn instance_line(note: InstanceNote) -> BackendNote {
    let (event, kind) = match note {
        InstanceNote::StaleSocketKept(kind) => ("instance.stale_socket_kept", kind),
        InstanceNote::Unbound(kind) => ("instance.listen_failed", kind),
        InstanceNote::AcceptFailed(kind) => ("instance.accept_failed", kind),
    };
    BackendNote {
        level: DiagnosticLogLevel::Warning,
        event,
        properties: vec![("kind", error_kind(kind).into())],
        idle: None,
        lifecycle: None,
        lock_reporting: None,
    }
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
            idle: Some(backend_name(backend)),
            lifecycle: None,
            lock_reporting: None,
        },
        IdleNote::Changed { from, to } => BackendNote {
            level: level(to),
            event: "idle.backend_changed",
            properties: vec![("from", name(from)), ("to", name(to))],
            idle: Some(backend_name(to)),
            lifecycle: None,
            lock_reporting: None,
        },
        IdleNote::InhibitorsCount => BackendNote {
            level: DiagnosticLogLevel::Information,
            event: "idle.inhibitor_blind",
            properties: vec![("value", false.into())],
            idle: None,
            lifecycle: None,
            lock_reporting: None,
        },
    }
}

fn backend_name(backend: Option<Backend>) -> &'static str {
    backend.map_or(NO_BACKEND, Backend::name)
}

/// A lifecycle source that tells the diagnostic log, and the export through `Backends`, when its
/// watch does not start, as logind then records no lock and no sleep, and whether the desktop
/// reports locks.
struct Reported<S> {
    source: S,
    notes: mpsc::UnboundedSender<BackendNote>,
}

impl<S: LifecycleSource> LifecycleSource for Reported<S> {
    fn snapshot(&self) -> SessionSnapshot {
        self.source.snapshot()
    }

    fn watch(&self, listener: Box<dyn LifecycleListener>) -> io::Result<Box<dyn Send>> {
        let listener = Box::new(Reporting {
            listener,
            notes: self.notes.clone(),
        });
        self.source.watch(listener).inspect_err(|error| {
            self.notes.unbounded_send(watch_failed(error)).ok();
        })
    }
}

/// This copy's entry in the menu of applications, which tells the diagnostic log when it cannot
/// be written.
struct ReportedLauncher {
    entry: LauncherEntry,
    notes: mpsc::UnboundedSender<BackendNote>,
}

impl ReportedLauncher {
    fn reported(&self, written: io::Result<()>) -> io::Result<()> {
        written.inspect_err(|error| {
            self.notes.unbounded_send(launcher_failed(error)).ok();
        })
    }
}

impl LauncherPort for ReportedLauncher {
    fn is_added(&self) -> bool {
        self.entry.is_added()
    }

    fn set_added(&self, added: bool) -> io::Result<()> {
        self.reported(self.entry.set_added(added))
    }

    fn settle(&self, first_run: bool) -> io::Result<()> {
        self.reported(self.entry.settle(first_run))
    }
}

/// The diagnostic log's line for a menu entry not written. The error's text can name the folder.
fn launcher_failed(error: &io::Error) -> BackendNote {
    BackendNote {
        level: DiagnosticLogLevel::Warning,
        event: "launcher_entry.failed",
        properties: vec![("kind", error_kind(error.kind()).into())],
        idle: None,
        lifecycle: None,
        lock_reporting: None,
    }
}

/// Hands each change on to `listener`, and tells the log whether the desktop reports locks.
struct Reporting {
    listener: Box<dyn LifecycleListener>,
    notes: mpsc::UnboundedSender<BackendNote>,
}

impl LifecycleListener for Reporting {
    fn changed(&mut self, transition: LifecycleTransition) {
        self.listener.changed(transition);
    }

    fn ended(&mut self) {
        self.listener.ended();
    }

    fn lock_reporting(&mut self, reporting: LockReporting) {
        self.notes.unbounded_send(lock_line(reporting)).ok();
    }
}

/// The diagnostic log's line for whether the desktop reports locks (contracts/linux-backends.md,
/// « Lock, sleep, session »). Without it the day rests on activity alone: a warning.
fn lock_line(reporting: LockReporting) -> BackendNote {
    BackendNote {
        level: match reporting {
            LockReporting::Live => DiagnosticLogLevel::Information,
            LockReporting::Unavailable => DiagnosticLogLevel::Warning,
        },
        event: "lifecycle.lock_reporting",
        properties: vec![("value", lock_reporting_name(reporting).into())],
        idle: None,
        lifecycle: None,
        lock_reporting: Some(reporting),
    }
}

/// The diagnostic log's line for a lifecycle watch that did not start (contracts/linux-backends.md,
/// « Lock, sleep, session »).
fn watch_failed(error: &io::Error) -> BackendNote {
    BackendNote {
        level: DiagnosticLogLevel::Warning,
        event: "lifecycle.watch_failed",
        properties: vec![("kind", error_kind(error.kind()).into())],
        idle: None,
        lifecycle: Some(NO_BACKEND),
        lock_reporting: Some(LockReporting::Unavailable),
    }
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
        if let Some(x11) = desktop.as_deref().and_then(LinuxDesktop::x11) {
            let _ = act(x11, id);
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
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use bientot_core::diagnostics::DiagnosticLogLevel::{Error, Information, Warning};
    use bientot_domain::accounting::SessionState;
    use bientot_linux::idle::{Backend, IdleNote};
    use bientot_linux::launcher::LauncherEntry;
    use bientot_linux::notifications::Service;
    use bientot_linux::single_instance::InstanceNote;
    use bientot_platform::launcher::LauncherPort;
    use bientot_platform::lifecycle::{
        InMemoryLifecycle, LifecycleChange, LifecycleListener, LifecycleSource,
        LifecycleTransition, LockReporting, SessionSnapshot,
    };
    use bientot_platform::notifications::{NotificationRequest, Notifier as _, QueuedClicks};
    use futures::channel::mpsc;

    use super::{BackendNote, Reported, ReportedLauncher, idle_line, instance_line, notifications};

    /// The line of a show the desktop refused, as Windows logs a refused toast.
    fn refused_toast_line() -> BackendNote {
        BackendNote {
            level: Error,
            event: "notification.failed",
            properties: vec![(
                "exceptionType",
                "bientot_core::notifications::NotificationError".into(),
            )],
            idle: None,
            lifecycle: None,
            lock_reporting: None,
        }
    }

    #[test]
    fn a_refused_show_is_logged_as_a_refused_toast_is_on_windows() {
        assert_eq!(super::notification_failed(), refused_toast_line());
    }

    /// python-dbusmock's notification server, on the bus of `scripts/linux-desktop-harness.sh`,
    /// refusing `Notify` for the life of the value.
    struct Refusing;

    impl Refusing {
        fn new() -> Self {
            answer_notify_with(
                "raise dbus.exceptions.DBusException('refused', name='org.freedesktop.DBus.Error.Failed')",
            );
            Self
        }
    }

    impl Drop for Refusing {
        fn drop(&mut self) {
            answer_notify_with(
                "if args[1]:\n    ret = args[1]\nelse:\n    ret = self.next_id\n    self.next_id += 1\n",
            );
        }
    }

    fn answer_notify_with(code: &str) {
        let output = std::process::Command::new("busctl")
            .args([
                "--user",
                "call",
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.DBus.Mock",
                "AddMethod",
                "sssss",
                "org.freedesktop.Notifications",
                "Notify",
                "susssasa{sv}i",
                "u",
                code,
            ])
            .output()
            .expect("busctl runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "desktop-harness"]
    fn desktop_harness_a_show_the_server_refuses_reaches_the_diagnostic_log() {
        let (notes, mut sent) = mpsc::unbounded();
        let shown = notifications(Service::Server, QueuedClicks::default(), notes).unwrap();
        let _refusing = Refusing::new();

        shown
            .show(&NotificationRequest {
                delivery_id: "refused".into(),
                title: "Bientôt".into(),
                body: "Il reste 15 minutes avant votre objectif.".into(),
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let note = loop {
            if let Ok(note) = sent.try_recv() {
                break note;
            }
            assert!(Instant::now() < deadline, "gave up waiting for the refusal");
            std::thread::sleep(Duration::from_millis(50));
        };

        assert_eq!(note, refused_toast_line());
    }

    #[test]
    fn a_menu_entry_that_cannot_be_written_is_logged_with_its_kind() {
        let not_a_folder = tempfile::NamedTempFile::new().unwrap();
        let (notes, mut sent) = mpsc::unbounded();
        let launcher = ReportedLauncher {
            entry: LauncherEntry::new(
                not_a_folder.path(),
                std::path::Path::new("/opt/Bientot.AppImage"),
                None,
            )
            .unwrap(),
            notes,
        };

        assert!(launcher.settle(true).is_err());
        assert!(launcher.set_added(true).is_err());

        let line = BackendNote {
            level: Warning,
            event: "launcher_entry.failed",
            properties: vec![("kind", "other".into())],
            idle: None,
            lifecycle: None,
            lock_reporting: None,
        };
        assert_eq!(
            [sent.try_recv().ok(), sent.try_recv().ok()],
            [Some(line.clone()), Some(line)]
        );
    }

    #[test]
    fn what_keeps_later_launches_away_is_logged_with_its_kind() {
        let line = |event, kind: &'static str| BackendNote {
            level: Warning,
            event,
            properties: vec![("kind", kind.into())],
            idle: None,
            lifecycle: None,
            lock_reporting: None,
        };
        assert_eq!(
            [
                InstanceNote::StaleSocketKept(io::ErrorKind::PermissionDenied),
                InstanceNote::Unbound(io::ErrorKind::AddrInUse),
                InstanceNote::Unbound(io::ErrorKind::InvalidInput),
                InstanceNote::AcceptFailed(io::ErrorKind::NotFound),
                InstanceNote::AcceptFailed(io::ErrorKind::OutOfMemory),
            ]
            .map(instance_line),
            [
                line("instance.stale_socket_kept", "permission_denied"),
                line("instance.listen_failed", "address_in_use"),
                line("instance.listen_failed", "invalid_input"),
                line("instance.accept_failed", "not_found"),
                line("instance.accept_failed", "other"),
            ]
        );
    }

    #[test]
    fn the_log_and_the_export_name_the_idle_backend_chosen_then_each_change() {
        let line = |level, event, properties: &[(&'static str, &'static str)], idle| BackendNote {
            level,
            event,
            properties: properties
                .iter()
                .map(|&(name, value)| (name, value.into()))
                .collect(),
            idle: Some(idle),
            lifecycle: None,
            lock_reporting: None,
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
                line(
                    Information,
                    "idle.backend",
                    &[("name", "MIT-SCREEN-SAVER")],
                    "MIT-SCREEN-SAVER",
                ),
                line(
                    Warning,
                    "idle.backend_changed",
                    &[("from", "MIT-SCREEN-SAVER"), ("to", "none")],
                    "none",
                ),
                line(Warning, "idle.backend", &[("name", "none")], "none"),
                line(
                    Information,
                    "idle.backend_changed",
                    &[("from", "none"), ("to", "org.gnome.Mutter.IdleMonitor")],
                    "org.gnome.Mutter.IdleMonitor",
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
                idle: None,
                lifecycle: None,
                lock_reporting: None,
            }
        );
    }

    /// A source whose watch fails as logind's does without a system bus or a session.
    struct Refused(io::ErrorKind);

    impl LifecycleSource for Refused {
        fn snapshot(&self) -> SessionSnapshot {
            unreachable!()
        }

        fn watch(&self, _: Box<dyn LifecycleListener>) -> io::Result<Box<dyn Send>> {
            Err(io::Error::new(self.0, "no session for someone@host"))
        }
    }

    struct Ignored;

    impl LifecycleListener for Ignored {
        fn changed(&mut self, _: LifecycleTransition) {}

        fn ended(&mut self) {}
    }

    #[test]
    fn a_watch_that_does_not_start_logs_its_kind_and_leaves_no_lifecycle_backend_nor_locks() {
        let line = |kind: &'static str| BackendNote {
            level: Warning,
            event: "lifecycle.watch_failed",
            properties: vec![("kind", kind.into())],
            idle: None,
            lifecycle: Some("none"),
            lock_reporting: Some(LockReporting::Unavailable),
        };
        for (kind, logged) in [
            (io::ErrorKind::NotFound, "not_found"),
            (io::ErrorKind::PermissionDenied, "permission_denied"),
            (io::ErrorKind::Other, "other"),
            (io::ErrorKind::ConnectionRefused, "other"),
        ] {
            let (notes, mut sent) = mpsc::unbounded();
            let source = Reported {
                source: Refused(kind),
                notes,
            };

            assert!(source.watch(Box::new(Ignored)).is_err(), "{kind:?}");
            assert_eq!(sent.try_recv().ok(), Some(line(logged)), "{kind:?}");
        }
    }

    #[test]
    fn a_watch_that_starts_logs_nothing() {
        let (notes, mut sent) = mpsc::unbounded();
        let source = Reported {
            source: InMemoryLifecycle::default(),
            notes,
        };

        let _watch = source.watch(Box::new(Ignored)).unwrap();
        assert_eq!(sent.try_recv().ok(), None);
    }

    /// A watch that says the desktop reports no lock, then starts reporting them with a lock,
    /// as logind's does when a screensaver starts after the app.
    struct LateScreensaver(LifecycleTransition);

    impl LifecycleSource for LateScreensaver {
        fn snapshot(&self) -> SessionSnapshot {
            unreachable!()
        }

        fn watch(&self, mut listener: Box<dyn LifecycleListener>) -> io::Result<Box<dyn Send>> {
            listener.lock_reporting(LockReporting::Unavailable);
            listener.lock_reporting(LockReporting::Live);
            listener.changed(self.0.clone());
            Ok(Box::new(()))
        }
    }

    struct Collected(Arc<Mutex<Vec<LifecycleTransition>>>);

    impl LifecycleListener for Collected {
        fn changed(&mut self, transition: LifecycleTransition) {
            self.0.lock().unwrap().push(transition);
        }

        fn ended(&mut self) {}
    }

    #[test]
    fn the_log_and_the_export_follow_whether_the_desktop_reports_locks() {
        let lock = LifecycleTransition {
            change: LifecycleChange::Session(SessionState::Locked),
            occurred_at: "2026-11-27T09:00:00Z".parse().unwrap(),
            time_zone_id: "Europe/Paris".into(),
        };
        let (notes, sent) = mpsc::unbounded();
        let recorded = Arc::default();
        let source = Reported {
            source: LateScreensaver(lock.clone()),
            notes,
        };

        let _watch = source
            .watch(Box::new(Collected(Arc::clone(&recorded))))
            .unwrap();
        drop(source);
        let line = |level, value: &'static str, reporting| BackendNote {
            level,
            event: "lifecycle.lock_reporting",
            properties: vec![("value", value.into())],
            idle: None,
            lifecycle: None,
            lock_reporting: Some(reporting),
        };
        assert_eq!(
            futures::executor::block_on_stream(sent).collect::<Vec<_>>(),
            [
                line(Warning, "unavailable", LockReporting::Unavailable),
                line(Information, "live", LockReporting::Live),
            ]
        );
        assert_eq!(
            *recorded.lock().unwrap(),
            [lock],
            "the ledger still gets the lock"
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
