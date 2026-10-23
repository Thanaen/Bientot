// Release builds on Windows have no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;

use anyhow::Context as _;
use worktime_app::assets::{self, Assets};
use worktime_app::demo::DemoHost;
use worktime_app::host::{self, Source, Today};
#[cfg(windows)]
use worktime_app::launch::Activation;
use worktime_app::launch::{Launch, Origin};
use worktime_app::live::{self, Announcement, LiveHost};
use worktime_app::startup_failure::StartupFailure;
use worktime_app::strings::{Key, Language};
use worktime_app::theme::ThemeMode;
use worktime_app::updates::UpdateSource;
use worktime_app::zones::SystemZones;
use worktime_app::{
    adjust_view, flyout, onboarding, onboarding_view, palette_view, settings_view, shell,
    startup_failure_view, text_input, transfer, updates, usage_ping, week_view,
};
use worktime_core::diagnostics::DiagnosticLogLevel;
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::time::SystemClock;
use worktime_store::data_directory::DataDirectory;
use worktime_store::settings::{LoadedSettings, SettingsSource, SettingsStore};
#[cfg(windows)]
use worktime_windows::single_instance::{Instance, PrimaryInstance};
use worktime_windows::startup::LaunchIntent;

static CLOCK: SystemClock = SystemClock;
static ZONES: SystemZones = SystemZones;
#[cfg(windows)]
static INPUT: worktime_windows::activity::SystemLastInput =
    worktime_windows::activity::SystemLastInput;
#[cfg(not(windows))]
static INPUT: NoLastInput = NoLastInput;

/// Only Windows reports the last input.
#[cfg(not(windows))]
struct NoLastInput;

#[cfg(not(windows))]
impl worktime_windows::activity::LastInputProbe for NoLastInput {
    fn read(&self) -> Option<worktime_domain::sources::LastInputReading> {
        None
    }
}

fn main() -> anyhow::Result<()> {
    // Before anything else: the uninstaller's cleanup, which then ends the process.
    #[cfg(all(windows, feature = "updates"))]
    let may_install = updates::direct::run_hooks();
    let launch = Launch::parse(std::env::args().skip(1))?;
    let Err(StartFailure {
        error,
        directory,
        #[cfg(windows)]
        instance,
    }) = run(
        launch.clone(),
        #[cfg(all(windows, feature = "updates"))]
        may_install,
    )
    else {
        return Ok(());
    };
    let failure = StartupFailure::new(&error, directory.as_ref(), launch.language);
    application().run(move |cx| {
        prepare(launch.theme, cx);
        #[cfg(windows)]
        if let Some(instance) = instance {
            native::listen(instance, cx);
        }
        fail(failure, cx);
    });
    Err(error)
}

/// A start that failed, the data folder it was on when there is one to export from, and the
/// folder's instance once claimed: a later launch then brings the failure to the front.
struct StartFailure {
    error: anyhow::Error,
    directory: Option<DataDirectory>,
    #[cfg(windows)]
    instance: Option<PrimaryInstance>,
}

/// WorkTime's app. A process runs only one: `run` builds it once `run_on` succeeded, so a start
/// that failed builds one for its window.
fn application() -> gpui::Application {
    gpui_platform::application().with_assets(Assets)
}

/// What every window of the app needs: the theme, the motion, the fonts and the keys.
fn prepare(theme: ThemeMode, cx: &mut gpui::App) {
    cx.set_global(theme);
    #[cfg(windows)]
    native::follow_animation_effects(cx);
    if let Err(error) = assets::load_fonts(cx) {
        log::error!("cannot load the fonts: {error:#}");
    }
    flyout::bind_keys(cx);
    week_view::bind_keys(cx);
    palette_view::bind_keys(cx);
    adjust_view::bind_keys(cx);
    settings_view::bind_keys(cx);
    onboarding_view::bind_keys(cx);
    startup_failure_view::bind_keys(cx);
    text_input::bind_keys(cx);
}

/// Shows why WorkTime does not start, in a window that offers the diagnostic ZIP, as the .NET
/// main window does; a message box when that window cannot open.
fn fail(failure: StartupFailure, cx: &mut gpui::App) {
    let language = failure.language();
    if let Err(error) = startup_failure_view::open(failure, cx) {
        log::error!("cannot show the startup failure: {error:#}");
        alert(language, Key::InitializationFailed);
        cx.quit();
    }
}

fn run(
    launch: Launch,
    #[cfg(all(windows, feature = "updates"))] may_install: bool,
) -> Result<(), StartFailure> {
    let directory = launch
        .data_directory(local_app_data)
        .map_err(|error| StartFailure {
            error,
            directory: None,
            #[cfg(windows)]
            instance: None,
        })?;
    // A demo writes no log: there is nothing in its folder to export.
    let exportable = launch.demo.is_none();
    // A restart for an import waits for the WorkTime it replaces to give up the folder.
    transfer::wait_for_restart(launch.restarted_from);
    #[cfg(windows)]
    let instance = match worktime_windows::single_instance::claim(&directory) {
        Ok(Instance::Primary(instance)) => instance,
        Ok(Instance::Secondary) => return Ok(()),
        Err(error) => {
            return Err(StartFailure {
                error: error.into(),
                directory: exportable.then_some(directory),
                instance: None,
            });
        }
    };
    // Only the user's own WorkTime installs the update it downloaded: the updater stops every
    // process of the installed build, a demo's or another folder's included.
    #[cfg(all(windows, feature = "updates"))]
    if may_install && launch.demo.is_none() && launch.data_dir.is_none() {
        updates::direct::install_pending();
    }
    // Before anything opens the ledger. A demo's folder is thrown away: nothing is staged there.
    let imported = match launch.demo {
        Some(_) => None,
        None => transfer::apply_staged(&directory),
    };
    let theme = launch.theme;
    let boot = match run_on(launch, &directory, imported) {
        Ok(boot) => boot,
        Err(error) => {
            return Err(StartFailure {
                error,
                directory: exportable.then_some(directory),
                #[cfg(windows)]
                instance: Some(instance),
            });
        }
    };
    let app = application();
    // The notification-area icon keeps the app running after its flyout, the only window, closes.
    #[cfg(windows)]
    let app = app.with_quit_mode(gpui::QuitMode::Explicit);
    app.run(move |cx| {
        prepare(theme, cx);
        #[cfg(windows)]
        native::listen(instance, cx);
        boot(directory, exportable, cx);
    });
    Ok(())
}

/// Readies WorkTime's start on `directory`. What it hands back starts it once the app runs, on
/// that same folder: the onboarding on a first run, else the day. A start that fails after the
/// onboarding offers to export the folder when it is `exportable`. The first flyout tells how
/// the import staged before a restart went, when there was one.
fn run_on(
    launch: Launch,
    directory: &DataDirectory,
    imported: Option<Announcement>,
) -> anyhow::Result<impl FnOnce(DataDirectory, bool, &mut gpui::App) + 'static> {
    // Before any window, the onboarding's included, as `Toasts::register` requires. A demo shows
    // no toasts, as in .NET.
    #[cfg(windows)]
    let notifications = launch.demo.is_none().then(|| {
        let data_dir = launch.data_dir.as_ref().map(|_| directory.path());
        native::Notifications::register(data_dir, launch.origin)
    });
    #[cfg(not(windows))]
    let notifications = None;
    // A demo reads its settings as it opens.
    let loaded = match launch.demo {
        Some(_) => None,
        None => Some(live::load_settings(directory)?),
    };
    let saved = loaded
        .as_ref()
        .is_none_or(|loaded| loaded.source != SettingsSource::Missing);
    let first_run = onboarding::should_show(saved, launch.demo.is_some(), launch.onboarding);
    #[cfg(windows)]
    let activation = worktime_windows::startup::current_activation();
    #[cfg(not(windows))]
    let activation = worktime_windows::startup::Activation::Launch;
    let intent = LaunchIntent::resolve(launch.windows_startup, &activation);
    let open_flyout = launch.open_flyout
        || launch.origin == Origin::ToastActivator
        || intent == LaunchIntent::Manual
        || imported.is_some();
    let may_send = launch.may_send_usage_ping();
    let update_source = UpdateSource::of(&launch, settings_view::build().channel());
    // As in .NET, nothing else starts before the onboarding is saved: the day, the tray and
    // the refreshes all begin from its choices.
    let (started, loaded, notifications) = if first_run {
        (None, loaded, notifications)
    } else {
        (
            Some(start(directory, &launch, loaded, notifications, imported)?),
            None,
            None,
        )
    };
    let store = SettingsStore::new(directory.settings_file())?;
    // A demo leaves the user's registration alone, as in .NET.
    #[cfg(windows)]
    let startup = launch.demo.is_none().then(|| {
        let registration = worktime_windows::startup::StartupRegistration::for_current_process();
        registration.migrate_owned_legacy_registration();
        registration
    });

    let boot = move |directory: DataDirectory, exportable: bool, cx: &mut gpui::App| {
        #[cfg(windows)]
        if let Some(startup) = startup {
            cx.set_global(settings_view::Startup(std::sync::Arc::new(startup)));
        }
        if let Some(started) = started {
            show(started, open_flyout, may_send, update_source, cx);
            return;
        }
        let settings = loaded.map_or_else(WorkTimeSettings::default, |loaded| loaded.settings);
        let save: onboarding_view::SaveSettings = match launch.demo {
            Some(_) => std::sync::Arc::new(|_| Ok(())),
            None => std::sync::Arc::new(move |settings| store.save(settings)),
        };
        let forced = launch.language;
        // A demo's folder is thrown away, so it offers no import.
        let import = launch.demo.is_none().then(|| directory.clone());
        onboarding_view::open(
            settings,
            forced,
            import,
            save,
            Box::new(move |settings, cx| {
                let loaded = LoadedSettings {
                    settings,
                    source: SettingsSource::File,
                };
                match start(&directory, &launch, Some(loaded), notifications, imported) {
                    // As .NET starts on `AppLaunchIntent.Manual` after the onboarding.
                    Ok(started) => show(started, true, may_send, update_source, cx),
                    Err(error) => {
                        log::error!("cannot start after the onboarding: {error:#}");
                        let exported = exportable.then_some(&directory);
                        fail(StartupFailure::new(&error, exported, forced), cx);
                    }
                }
            }),
            cx,
        );
    };
    Ok(boot)
}

/// The day opened, and what Windows tells about it until the shell listens.
struct Started {
    today: Today,
    language: Language,
    #[cfg(windows)]
    lifecycle: Option<native::Lifecycle>,
    #[cfg(windows)]
    toast_clicks: Option<futures::channel::mpsc::UnboundedReceiver<Activation>>,
}

/// Opens the day on `loaded`, the settings read or just saved by the onboarding. A demo opens
/// its own settings, replaced by the onboarding's when given. A live day shows its reminders
/// through `notifications`, registered before any window opened, and its first flyout tells
/// `imported`.
fn start(
    directory: &DataDirectory,
    launch: &Launch,
    loaded: Option<LoadedSettings>,
    #[cfg_attr(
        not(windows),
        allow(unused_variables, reason = "only Windows shows toasts")
    )]
    notifications: Option<native::Notifications>,
    imported: Option<Announcement>,
) -> anyhow::Result<Started> {
    #[cfg(windows)]
    let (mut lifecycle, mut toast_clicks) = (None, None);
    let (source, language) = match launch.demo {
        Some(scenario) => {
            let mut host = DemoHost::open(scenario, directory.path())?;
            if let Some(loaded) = loaded {
                host.save_settings(&loaded.settings)?;
            }
            let language = Language::resolve(
                launch.language,
                host.settings().language_override.as_deref(),
            );
            (Source::Demo(Box::new(host)), language)
        }
        None => {
            let loaded = match loaded {
                Some(loaded) => loaded,
                None => live::load_settings(directory)?,
            };
            let language = Language::resolve(
                launch.language,
                loaded.settings.language_override.as_deref(),
            );
            // Listens before reading what Windows saw, as `WindowsLifecyclePorts.Attach` does:
            // a change meanwhile waits for the startup state.
            #[cfg(windows)]
            let mut watching = native::Lifecycle::watch();
            // The ledger lives as long as the process.
            let ledger = Box::leak(Box::new(live::open_ledger(directory)?));
            let mut host = LiveHost::start(directory, loaded, ledger, &CLOCK, &ZONES, &INPUT)?;
            if let Some(imported) = imported {
                host.announce(imported);
            }
            #[cfg(windows)]
            {
                native::catch_up(&mut host, &mut watching)?;
                lifecycle = Some(watching);
                toast_clicks =
                    notifications.map(|notifications| notifications.enable(&mut host, language));
            }
            host.poll();
            (Source::Live(Box::new(host)), language)
        }
    };
    Ok(Started {
        today: Today::open(source, language)?,
        language,
        #[cfg(windows)]
        lifecycle,
        #[cfg(windows)]
        toast_clicks,
    })
}

/// Makes `started` the shell's day: the tray, the flyout and the timers act on it from now on.
/// The usage signal goes only when `may_send`; the flyout offers the updates of `update_source`.
fn show(
    started: Started,
    open_flyout: bool,
    may_send: bool,
    update_source: Option<UpdateSource>,
    cx: &mut gpui::App,
) {
    cx.set_global(started.language);
    host::install(started.today, cx);
    if may_send {
        usage_ping::install(cx);
    }
    #[cfg(windows)]
    native::attach(started.lifecycle, started.toast_clicks, cx);
    // After the tray: its window owns the Store's prompts.
    updates::start(update_source, cx);
    if open_flyout {
        shell::open_flyout(cx);
    }
    if let Some(host) = cx.global_mut::<Today>().live() {
        // scripts/measure-performance.ps1 times the startup to this event.
        host.log(DiagnosticLogLevel::Information, "startup.tray_ready", &[]);
    }
    // The CI startup measurement waits for this file: the shell is ready to be clicked.
    if let Some(path) = std::env::var_os("WORKTIME_READY_FILE") {
        std::fs::write(path, b"").ok();
    }
}

/// `%LOCALAPPDATA%`, where the .NET app keeps `WorkTime`; the XDG data folder elsewhere.
fn local_app_data() -> anyhow::Result<PathBuf> {
    let variable = |name| std::env::var_os(name).map(PathBuf::from);
    if cfg!(windows) {
        variable("LOCALAPPDATA")
    } else {
        variable("XDG_DATA_HOME").or_else(|| variable("HOME").map(|home| home.join(".local/share")))
    }
    .context("no folder for the application data")
}

/// Tells the user why WorkTime does not start. Release builds on Windows have no console.
fn alert(language: Language, key: Key) {
    let text = language.text(key);
    eprintln!("{text}");
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
        use windows::core::{HSTRING, PCWSTR};
        let (text, title) = (
            HSTRING::from(text),
            HSTRING::from(language.text(Key::ShellWindowTitle)),
        );
        // SAFETY: both strings outlive the modal call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONWARNING,
            )
        };
    }
}

/// Only Windows shows toasts.
#[cfg(not(windows))]
mod native {
    pub enum Notifications {}
}

/// The Windows notifications the shell listens to: tray clicks, later launches for the same
/// folder, the session and power changes of a live day, and the « Animation effects » setting.
#[cfg(windows)]
mod native {
    use std::path::Path;

    use futures::StreamExt as _;
    use futures::channel::mpsc;
    use gpui::{App, Global};
    use worktime_app::host::{self, Today};
    use worktime_app::launch::{Activation, Activations, Origin};
    use worktime_app::live::LiveHost;
    use worktime_app::notify::ToastSink;
    use worktime_app::strings::Language;
    use worktime_app::zones::SystemZones;
    use worktime_app::{onboarding_view, shell, startup_failure_view};
    use worktime_core::diagnostics::DiagnosticLogLevel;
    use worktime_domain::time::SystemClock;
    use worktime_windows::animation_effects::{self, SettingChanges};
    use worktime_windows::lifecycle::{
        LifecycleEventRecorder, LifecycleTransition, LifecycleWindow, current_session_state,
        probe_within,
    };
    use worktime_windows::notifications::{Sender, Toasts};
    use worktime_windows::power_history::{PowerHistoryReader, RECORD_LIMIT, SystemLog};
    use worktime_windows::single_instance::{ActivationListener, PrimaryInstance};

    /// The session and power changes of a live day, queued in `arrived` from the moment the
    /// window opens: its own thread receives them, before GPUI runs.
    pub struct Lifecycle {
        window: Option<LifecycleWindow>,
        arrived: mpsc::UnboundedReceiver<LifecycleTransition>,
    }

    impl Lifecycle {
        pub fn watch() -> Self {
            let (transitions, arrived) = mpsc::unbounded();
            let window = LifecycleWindow::open(SystemClock, SystemZones, move |transition| {
                transitions.unbounded_send(transition).ok();
            })
            .inspect_err(|error| log::error!("cannot watch the session and power: {error}"))
            .ok();
            Self { window, arrived }
        }
    }

    /// Records what Windows saw while WorkTime was not running, the session state now, then
    /// the changes queued meanwhile, before the first summary.
    pub fn catch_up(host: &mut LiveHost<'static>, lifecycle: &mut Lifecycle) -> anyhow::Result<()> {
        host.recover_power_history(|window| {
            PowerHistoryReader::new(SystemLog, RECORD_LIMIT).read(window)
        })?;
        host.prepare_lifecycle(
            || {
                probe_within(
                    LifecycleEventRecorder::PREPARE_TIMEOUT,
                    current_session_state,
                )
            },
            std::iter::from_fn(|| lifecycle.arrived.try_recv().ok()),
        );
        Ok(())
    }

    /// WorkTime's toasts, registered with Windows, and the clicks on them as they arrive.
    pub struct Notifications {
        toasts: windows::core::Result<Toasts>,
        clicks: mpsc::UnboundedReceiver<Activation>,
    }

    impl Notifications {
        /// Registers the toasts of this process, which a click starts on `data_dir` when given.
        /// A click queues until the shell listens, so the one COM started this process for
        /// waits for the day, after the onboarding on a first run.
        pub fn register(data_dir: Option<&Path>, origin: Origin) -> Self {
            let activations = Activations::of(origin);
            let (arrived, clicks) = mpsc::unbounded();
            let toasts = Sender::current(data_dir).and_then(|sender| {
                Toasts::register(sender, move |_| {
                    arrived.unbounded_send(activations.arrived()).ok();
                })
            });
            Self { toasts, clicks }
        }

        /// Has the host show its reminders through the toasts, as `App.xaml.cs` does outside a
        /// demo, and hands back the clicks.
        ///
        /// Windows shows a toast only for a sender it knows: the MSIX package, or the
        /// AppUserModelID the portable build registers. A failed registration shows no reminder,
        /// so that none is reserved in the ledger without being shown, and the flyout says so.
        pub fn enable(
            self,
            host: &mut LiveHost<'static>,
            language: Language,
        ) -> mpsc::UnboundedReceiver<Activation> {
            match self.toasts {
                Ok(toasts) => host.enable_notifications(Box::leak(Box::new(ToastSink {
                    toasts,
                    language,
                    zones: &SystemZones,
                }))),
                Err(error) => host.notifications_refused(&error),
            }
            self.clicks
        }
    }

    /// Owns the data folder until the app exits.
    struct Listening {
        _listener: Option<ActivationListener>,
    }

    impl Global for Listening {}

    /// Owns the lifecycle window until the app exits.
    struct Held {
        _lifecycle: Option<LifecycleWindow>,
    }

    impl Global for Held {}

    /// Owns the settings watch until the app exits.
    struct Watching {
        _changes: Option<SettingChanges>,
    }

    impl Global for Watching {}

    /// Has GPUI show every animation at rest while the « Animation effects » setting is off,
    /// from now on and after each change of the system settings.
    pub fn follow_animation_effects(cx: &mut App) {
        // The hook runs inside a window procedure, where GPUI may hold the app: it only queues.
        let (changes, mut changed) = mpsc::unbounded();
        let watch = SettingChanges::watch(move || {
            changes.unbounded_send(()).ok();
        })
        .inspect_err(|error| log::error!("cannot watch the system settings: {error}"))
        .ok();
        // Read after the watch starts, so a change in between is not lost.
        cx.set_reduce_motion(animation_effects::reduce_motion());
        cx.spawn(async move |cx| {
            while changed.next().await.is_some() {
                cx.update(|cx| cx.set_reduce_motion(animation_effects::reduce_motion()));
            }
        })
        .detach();
        cx.set_global(Watching { _changes: watch });
    }

    /// Brings WorkTime to the front on a later launch for the same folder: the onboarding
    /// while it shows, as in .NET, or the window of a start that failed, else the flyout.
    pub fn listen(instance: PrimaryInstance, cx: &mut App) {
        let (activations, mut activated) = mpsc::unbounded();
        let listener = instance
            .listen(move || {
                activations.unbounded_send(()).ok();
            })
            .inspect_err(|error| log::error!("cannot listen for later launches: {error}"))
            .ok();
        cx.spawn(async move |cx| {
            while activated.next().await.is_some() {
                cx.update(|cx| {
                    if onboarding_view::activate(cx)
                        || startup_failure_view::activate(cx)
                        || !cx.has_global::<Today>()
                    {
                        return;
                    }
                    shell::open_flyout(cx);
                    if let Some(host) = cx.global_mut::<Today>().live() {
                        host.log(
                            DiagnosticLogLevel::Information,
                            "instance.redirected",
                            &[("activationKind", "Launch".into())],
                        );
                    }
                });
            }
        })
        .detach();
        cx.set_global(Listening {
            _listener: listener,
        });
    }

    /// The tray, the toasts and the lifecycle of the day the shell shows.
    pub fn attach(
        lifecycle: Option<Lifecycle>,
        toast_clicks: Option<mpsc::UnboundedReceiver<Activation>>,
        cx: &mut App,
    ) {
        if let Err(error) = worktime_app::tray::install(cx) {
            log::error!("cannot create the notification-area icon: {error:#}");
        }
        if let Some(mut clicked) = toast_clicks {
            cx.spawn(async move |cx| {
                while let Some(activation) = clicked.next().await {
                    cx.update(|cx| {
                        shell::open_flyout(cx);
                        if let Some(host) = cx.global_mut::<Today>().live() {
                            host.log(
                                DiagnosticLogLevel::Information,
                                "notification.activated",
                                &[("origin", activation.name().into())],
                            );
                        }
                    });
                }
            })
            .detach();
        }

        let mut window = None;
        if let Some(lifecycle) = lifecycle {
            let mut arrived = lifecycle.arrived;
            cx.spawn(async move |cx| {
                while let Some(transition) = arrived.next().await {
                    cx.update(|cx| {
                        host::change(cx, |today| {
                            today.with_live(|host| host.record_lifecycle(&transition));
                        })
                    });
                }
            })
            .detach();
            window = lifecycle.window;
        }
        cx.set_global(Held { _lifecycle: window });
    }
}
