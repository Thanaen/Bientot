// Release builds on Windows have no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::rc::Rc;

use bientot_app::assets::{self, Assets};
use bientot_app::demo::DemoHost;
use bientot_app::host::{self, Source, Today};
use bientot_app::launch::{Activation, Activations, Launch, Origin};
use bientot_app::live::{self, Announcement, LiveHost};
use bientot_app::notify::ToastSink;
use bientot_app::platform::{self, Platform, PlatformContext, TrayEvents};
use bientot_app::startup_failure::StartupFailure;
use bientot_app::strings::{Key, Language};
use bientot_app::theme::ThemeMode;
use bientot_app::tray_model::MenuCommand;
use bientot_app::updates::UpdateSourceKind;
use bientot_app::zones::SystemZones;
use bientot_app::{
    adjust_view, flyout, onboarding, onboarding_view, palette_view, settings_view, shell,
    startup_failure_view, text_input, transfer, updates, usage_ping, week_view,
};
use bientot_core::diagnostics::DiagnosticLogLevel;
use bientot_domain::configuration::Settings;
use bientot_domain::time::SystemClock;
use bientot_platform::lifecycle::{
    LifecycleEventRecorder, LifecycleListener, LifecycleSource, LifecycleTransition, probe_within,
};
use bientot_platform::motion::ReducedMotion;
use bientot_platform::notifications::Notifier;
use bientot_platform::placement::Desktop;
use bientot_platform::power_history::{PowerHistoryReader, RECORD_LIMIT};
use bientot_platform::session_history::{self, SessionHistoryReader};
use bientot_platform::single_instance::{Acquired, InstanceLock};
use bientot_platform::startup::LaunchIntent;
use bientot_platform::tray::Tray;
use bientot_store::data_directory::DataDirectory;
use bientot_store::data_migration::{Location, Migration};
use bientot_store::settings::{LoadedSettings, SettingsSource, SettingsStore};
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, Global, QuitMode};

static CLOCK: SystemClock = SystemClock;
static ZONES: SystemZones = SystemZones;

fn main() -> anyhow::Result<()> {
    // Before anything else: the uninstaller's cleanup, which then ends the process.
    let may_install = platform::run_update_hooks();
    let launch = Launch::parse(std::env::args().skip(1))?;
    let Err(StartFailure {
        error,
        directory,
        instance,
    }) = run(launch.clone(), may_install)
    else {
        return Ok(());
    };
    let failure = StartupFailure::new(&error, directory.as_ref(), launch.language);
    application().run(move |cx| {
        prepare(launch.theme, cx);
        if let Some(instance) = instance {
            listen(instance, cx);
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
    instance: Option<platform::Instance>,
}

/// WorkTime's app. A process runs only one: `run` builds it once `run_on` succeeded, so a start
/// that failed builds one for its window.
fn application() -> gpui::Application {
    gpui_platform::application().with_assets(Assets)
}

/// What every window of the app needs: the theme, the motion, the fonts and the keys.
fn prepare(theme: ThemeMode, cx: &mut App) {
    cx.set_global(theme);
    follow_reduced_motion(platform::motion(), cx);
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
fn fail(failure: StartupFailure, cx: &mut App) {
    let language = failure.language();
    if let Err(error) = startup_failure_view::open(failure, cx) {
        log::error!("cannot show the startup failure: {error:#}");
        alert(language, Key::InitializationFailed);
        cx.quit();
    }
}

fn run(launch: Launch, may_install: bool) -> Result<(), StartFailure> {
    let location = launch
        .location(platform::local_app_data)
        .map_err(|error| StartFailure {
            error,
            directory: None,
            instance: None,
        })?;
    // A demo writes no log: there is nothing in its folder to export.
    let exportable = launch.demo.is_none();
    // A restart for an import waits for the WorkTime it replaces to give up the folder.
    transfer::wait_for_restart(launch.restarted_from);
    let (directory, migration, claimed) = settle(location);
    let instance = match claimed {
        Ok(Acquired::Primary(instance)) => instance,
        Ok(Acquired::Secondary) => {
            if let Some(language) =
                transfer::import_left_waiting(launch.restarted_from, &directory, launch.language)
            {
                alert(language, Key::ImportWaitsForNextStart);
            }
            return Ok(());
        }
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
    if may_install && launch.demo.is_none() && launch.data_dir.is_none() {
        platform::install_pending_update();
    }
    // Before anything opens the ledger. A demo's folder is thrown away: nothing is staged there.
    let imported = match launch.demo {
        Some(_) => None,
        None => transfer::apply_staged(&directory),
    };
    let theme = launch.theme;
    let (quit_mode, boot) = match run_on(launch, &directory, imported, migration) {
        Ok(started) => started,
        Err(error) => {
            return Err(StartFailure {
                error,
                directory: exportable.then_some(directory),
                instance: Some(instance),
            });
        }
    };
    application().with_quit_mode(quit_mode).run(move |cx| {
        prepare(theme, cx);
        listen(instance, cx);
        boot(directory, exportable, cx);
    });
    Ok(())
}

/// The folder this launch runs on, how the move of the user's folder went, and the claim on the
/// folder. Before the move this launch claims the old folder: a WorkTime still running on it
/// keeps it and shows itself, and this launch moves nothing.
fn settle(
    location: Location,
) -> (
    DataDirectory,
    Option<Migration>,
    std::io::Result<Acquired<platform::Instance>>,
) {
    let claim = platform::Instance::acquire;
    let Some(old) = location.moving_from() else {
        let (directory, migration) = location.settle(|from, to| std::fs::rename(from, to));
        let claimed = claim(&directory);
        return (directory, migration, claimed);
    };
    let held = match claim(&old) {
        Ok(Acquired::Primary(held)) => held,
        other => return (old, None, other),
    };
    let (directory, migration) = location.settle(|from, to| std::fs::rename(from, to));
    let claimed = if directory == old {
        Ok(Acquired::Primary(held))
    } else {
        claim(&directory)
    };
    (directory, migration, claimed)
}

/// Readies WorkTime's start on `directory`. What it hands back starts it once the app runs, on
/// that same folder: the onboarding on a first run, else the day. A start that fails after the
/// onboarding offers to export the folder when it is `exportable`. The first flyout tells how
/// the import staged before a restart went, when there was one. It also says when the app
/// quits.
fn run_on(
    launch: Launch,
    directory: &DataDirectory,
    imported: Option<Announcement>,
    mut migration: Option<Migration>,
) -> anyhow::Result<(
    QuitMode,
    impl FnOnce(DataDirectory, bool, &mut App) + 'static,
)> {
    // Before any window, the onboarding's included: the notifications must be registered first.
    let mut platform = platform::build(&PlatformContext {
        directory,
        launch: &launch,
    })?;
    let quit_mode = shell::quit_mode(platform.tray.host());
    // A demo reads its settings as it opens.
    let loaded = match launch.demo {
        Some(_) => None,
        None => Some(live::load_settings(directory)?),
    };
    let saved = loaded
        .as_ref()
        .is_none_or(|loaded| loaded.source != SettingsSource::Missing);
    let first_run = onboarding::should_show(saved, launch.demo.is_some(), launch.onboarding);
    let open_flyout = launch.open_flyout
        || launch.origin == Origin::ToastActivator
        || platform.launch_intent == LaunchIntent::Manual
        || imported.is_some();
    let may_send = launch.may_send_usage_ping();
    let update_source = UpdateSourceKind::of(&launch, settings_view::build().channel());
    let startup = platform.startup.take();
    // As in .NET, nothing else starts before the onboarding is saved: the day, the tray and
    // the refreshes all begin from its choices.
    let opening = if first_run {
        Opening::Onboarding(loaded, platform)
    } else {
        Opening::Day(start(
            directory,
            &launch,
            loaded,
            platform,
            imported,
            migration.take(),
        )?)
    };
    let store = SettingsStore::new(directory.settings_file())?;

    let boot = move |directory: DataDirectory, exportable: bool, cx: &mut App| {
        if let Some(startup) = startup {
            cx.set_global(settings_view::Startup(startup));
        }
        let (loaded, platform) = match opening {
            Opening::Day(started) => {
                show(started, open_flyout, may_send, update_source, cx);
                return;
            }
            Opening::Onboarding(loaded, platform) => (loaded, platform),
        };
        let settings = loaded.map_or_else(Settings::default, |loaded| loaded.settings);
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
                let started = start(
                    &directory,
                    &launch,
                    Some(loaded),
                    platform,
                    imported,
                    migration,
                );
                match started {
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
    Ok((quit_mode, boot))
}

/// What the app opens on: the day, or on a first run the onboarding, with the settings read and
/// the platform the day starts on once it is saved.
#[allow(
    clippy::large_enum_variant,
    reason = "made once per launch and moved once into the app"
)]
enum Opening {
    Day(Started),
    Onboarding(Option<LoadedSettings>, Platform),
}

/// The day opened, and what the system tells about it until the shell listens.
struct Started {
    today: Today,
    language: Language,
    ports: ShellPorts,
    /// `false` with `--no-plugins`.
    #[cfg(feature = "plugins")]
    plugins: bool,
    lifecycle: Option<Lifecycle>,
    toast_clicks: Option<mpsc::UnboundedReceiver<Activation>>,
}

/// The tray the day shows in, and the desktop its flyout opens on.
struct ShellPorts {
    tray: Rc<dyn Tray<MenuCommand>>,
    tray_events: TrayEvents,
    tray_backend: Option<&'static str>,
    desktop: Rc<dyn Desktop>,
}

/// Opens the day on `loaded`, the settings read or just saved by the onboarding. A demo opens
/// its own settings, replaced by the onboarding's when given. A live day runs on `platform`,
/// whose notifications were registered before any window opened, and its first flyout tells
/// `imported`. Its log tells how the move of the user's folder went.
fn start(
    directory: &DataDirectory,
    launch: &Launch,
    loaded: Option<LoadedSettings>,
    platform: Platform,
    imported: Option<Announcement>,
    migration: Option<Migration>,
) -> anyhow::Result<Started> {
    let (mut lifecycle, mut toast_clicks) = (None, None);
    let ports = ShellPorts {
        tray: platform.tray,
        tray_events: platform.tray_events,
        tray_backend: platform.tray_backend,
        desktop: platform.desktop,
    };
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
            // Listens before reading what the system saw, as `WindowsLifecyclePorts.Attach`
            // does: a change meanwhile waits for the startup state.
            let mut watching = Lifecycle::watch(&*platform.lifecycle);
            // The ledger and the probe live as long as the process.
            let ledger = Box::leak(Box::new(live::open_ledger(directory)?));
            let input = Box::leak(platform.last_input);
            let mut host = LiveHost::start(directory, loaded, ledger, &CLOCK, &ZONES, input)?;
            if let Some(imported) = imported {
                host.announce(imported);
            }
            match migration {
                None => {}
                Some(Migration::Moved) => {
                    host.log(DiagnosticLogLevel::Information, "data_migration.moved", &[]);
                }
                Some(Migration::BothPresent) => {
                    host.log(
                        DiagnosticLogLevel::Warning,
                        "data_migration.both_present",
                        &[],
                    );
                }
                Some(Migration::Failed(error)) => host.log_failure("data_migration.failed", &error),
            }
            let Platform {
                lifecycle: source,
                power_log,
                session_log,
                notifier,
                notification_clicks,
                ..
            } = platform;
            host.recover_power_history(move |window| {
                PowerHistoryReader::new(power_log, RECORD_LIMIT).read(window)
            })?;
            host.recover_session_history(move |window| {
                SessionHistoryReader::new(session_log, session_history::RECORD_LIMIT).read(window)
            })?;
            catch_up(&mut host, source, &mut watching);
            lifecycle = Some(watching);
            enable_notifications(&mut host, notifier, language);
            toast_clicks = Some(take_clicks(&*notification_clicks, launch.origin));
            host.poll();
            (Source::Live(Box::new(host)), language)
        }
    };
    Ok(Started {
        today: Today::open(source, language)?,
        language,
        ports,
        #[cfg(feature = "plugins")]
        plugins: !launch.no_plugins,
        lifecycle,
        toast_clicks,
    })
}

/// Has `host` show its reminders through `notifier`, in `language`, or tells it that the system
/// refused them.
fn enable_notifications(
    host: &mut LiveHost<'static>,
    notifier: Option<Result<Box<dyn Notifier>, platform::NotificationsRefused>>,
    language: Language,
) {
    match notifier {
        Some(Ok(notifier)) => host.enable_notifications(Box::leak(Box::new(ToastSink {
            notifier,
            language,
            zones: &ZONES,
        }))),
        Some(Err(error)) => host.notifications_refused(&error),
        None => {}
    }
}

/// Makes `started` the shell's day: the tray, the flyout and the timers act on it from now on.
/// The usage signal goes only when `may_send`; the flyout offers the updates of `update_source`.
fn show(
    started: Started,
    open_flyout: bool,
    may_send: bool,
    update_source: Option<UpdateSourceKind>,
    cx: &mut App,
) {
    cx.set_global(started.language);
    host::install(started.today, cx);
    if may_send {
        usage_ping::install(cx);
    }
    attach(started.ports, started.lifecycle, started.toast_clicks, cx);
    // After the tray: its window owns the Store's prompts.
    updates::start(update_source, cx);
    // Without a tray icon nothing else shows the app, even started at sign-in.
    if open_flyout || shell::window_mode(cx) {
        shell::open_flyout(cx);
    }
    if let Some(host) = cx.global_mut::<Today>().live() {
        // scripts/measure-performance.ps1 times the startup to this event.
        host.log(DiagnosticLogLevel::Information, "startup.tray_ready", &[]);
    }
    // After the tray is ready, and only for the user's own day (plan, Lifecycle 2).
    #[cfg(feature = "plugins")]
    if let Some(directory) = cx
        .global_mut::<Today>()
        .live()
        .filter(|_| started.plugins)
        .map(|host| host.data_directory().clone())
    {
        let language = *cx.global::<Language>();
        let daily = matches!(
            update_source,
            Some(UpdateSourceKind::Store | UpdateSourceKind::Direct)
        );
        bientot_app::plugins::start(&directory, language, daily, cx);
    }
    // The CI startup measurement waits for this file: the shell is ready to be clicked.
    if let Some(path) = std::env::var_os("BIENTOT_READY_FILE") {
        std::fs::write(path, b"").ok();
    }
}

/// Tells the user why WorkTime does not start. Release builds on Windows have no console.
fn alert(language: Language, key: Key) {
    let text = language.text(key);
    eprintln!("{text}");
    platform::show_fatal(language.text(Key::ShellWindowTitle), text);
}

/// The session and power changes of a live day, queued in `arrived` from the moment the watch
/// starts: the source's own thread receives them, before GPUI runs.
struct Lifecycle {
    watch: Option<Box<dyn Send>>,
    arrived: mpsc::UnboundedReceiver<LifecycleTransition>,
}

impl Lifecycle {
    fn watch(source: &dyn LifecycleSource) -> Self {
        let (transitions, arrived) = mpsc::unbounded();
        let watch = source
            .watch(Box::new(Queue(transitions)))
            .inspect_err(|error| log::error!("cannot watch the session and power: {error}"))
            .ok();
        Self { watch, arrived }
    }
}

/// Queues the changes for the shell; closes the queue when the source ends.
struct Queue(mpsc::UnboundedSender<LifecycleTransition>);

impl LifecycleListener for Queue {
    fn changed(&mut self, transition: LifecycleTransition) {
        self.0.unbounded_send(transition).ok();
    }

    fn ended(&mut self) {
        self.0.close_channel();
    }
}

/// Records the session state now, then the changes queued meanwhile, before the first summary.
fn catch_up(
    host: &mut LiveHost<'static>,
    source: std::sync::Arc<dyn LifecycleSource + Send + Sync>,
    lifecycle: &mut Lifecycle,
) {
    host.prepare_lifecycle(
        || {
            probe_within(LifecycleEventRecorder::PREPARE_TIMEOUT, move || {
                source.snapshot().session
            })
        },
        std::iter::from_fn(|| lifecycle.arrived.try_recv().ok()),
    );
}

/// The clicks on the notifications, those that came before the day included, and how each
/// reached WorkTime.
fn take_clicks(
    clicks: &dyn bientot_platform::notifications::NotificationClicks,
    origin: Origin,
) -> mpsc::UnboundedReceiver<Activation> {
    let activations = Activations::of(origin);
    let (arrived, clicked) = mpsc::unbounded();
    clicks.on_click(Box::new(move |_| {
        arrived.unbounded_send(activations.arrived()).ok();
    }));
    clicked
}

/// Owns the data folder until the app exits.
struct Listening {
    _listener: Option<<platform::Instance as InstanceLock>::Listener>,
}

impl Global for Listening {}

/// Owns the lifecycle watch until the app exits.
struct Held {
    _lifecycle: Option<Box<dyn Send>>,
}

impl Global for Held {}

/// Owns the settings watch until the app exits.
struct Watching {
    _changes: Option<<platform::Motion as ReducedMotion>::Watch>,
}

impl Global for Watching {}

/// Has GPUI show every animation at rest while the system asks for less motion, from now on and
/// after each change of the setting.
fn follow_reduced_motion(motion: platform::Motion, cx: &mut App) {
    let (changes, mut changed) = mpsc::unbounded();
    let watch = motion
        .watch(Box::new(move || {
            changes.unbounded_send(()).ok();
        }))
        .inspect_err(|error| log::error!("cannot watch the system settings: {error}"))
        .ok();
    // Read after the watch starts, so a change in between is not lost.
    cx.set_reduce_motion(motion.current());
    cx.spawn(async move |cx| {
        while changed.next().await.is_some() {
            cx.update(|cx| cx.set_reduce_motion(motion.current()));
        }
    })
    .detach();
    cx.set_global(Watching { _changes: watch });
}

/// Brings WorkTime to the front on a later launch for the same folder: the onboarding while it
/// shows, as in .NET, or the window of a start that failed, else the flyout.
fn listen(instance: platform::Instance, cx: &mut App) {
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
fn attach(
    ShellPorts {
        tray,
        tray_events,
        tray_backend,
        desktop,
    }: ShellPorts,
    lifecycle: Option<Lifecycle>,
    toast_clicks: Option<mpsc::UnboundedReceiver<Activation>>,
    cx: &mut App,
) {
    shell::install(desktop, tray.host(), cx);
    host::install_tray(tray, tray_backend, tray_events, cx);
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

    let mut watch = None;
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
        watch = lifecycle.watch;
    }
    cx.set_global(Held { _lifecycle: watch });
}
