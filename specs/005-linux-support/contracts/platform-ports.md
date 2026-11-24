# Contract: platform ports

The traits of `bientot-platform` after Lots 1 and 2. Items marked
*(existing)* keep the signature they have on `main` at `d9f55ce` and only
change crate; items marked *(new)* are introduced by this feature. A lot may
rename a private helper; it may not change a signature below without
amending this file in the same pull request.

Every port has a Windows implementation, a Linux implementation and an
in-memory one for tests.

## Activity

```rust
// bientot-platform/src/activity.rs

/// (existing, from bientot-windows::activity)
pub trait LastInputProbe {
    /// `None` when the system cannot tell.
    fn read(&self) -> Option<LastInputReading>;
}

/// (existing) The reading for a `GetTickCount64` value and a `GetLastInputInfo` tick.
pub fn reading_from_ticks(tick_count64: u64, last_input_tick: u32) -> LastInputReading;

/// (new) The same reading for « the machine has been up for `uptime` and the
/// last input was `idle` ago ». `uptime` is monotonic and excludes sleep.
/// Wraps like the Windows tick, so the tracker needs no second code path.
pub fn reading_from_idle(uptime: Duration, idle: Duration) -> LastInputReading;

/// (existing, unchanged) BuiltInActivitySource<P: LastInputProbe, …>
```

`LastInputReading` stays in `bientot-domain`, untouched.

## Lifecycle

```rust
// bientot-platform/src/lib.rs
/// (existing, from bientot-windows)
pub enum PowerState { Active, Suspended }

// bientot-platform/src/lifecycle.rs
/// (existing)
pub enum LifecycleChange { Session(SessionState), Power(PowerState) }

/// (existing)
pub struct LifecycleTransition {
    pub change: LifecycleChange,
    pub occurred_at: Timestamp,
    pub time_zone_id: String,
}

/// (existing)
pub trait LifecycleHooks {
    fn power_changing(&mut self, state: PowerState);
    fn refresh(&mut self);
}

/// (new) What the platform knows about the session right now.
pub struct SessionSnapshot {
    pub session: SessionState,          // Unknown when the desktop does not report locks
    pub lock_reporting: LockReporting,
}

/// (new)
pub enum LockReporting {
    /// Lock and unlock are reported as they happen.
    Live,
    /// This desktop reports no lock state. Shown to the user (FR-011).
    Unavailable,
}

/// (new) Receives changes on the source's own thread, already stamped.
pub trait LifecycleListener: Send + 'static {
    fn changed(&mut self, transition: LifecycleTransition);
    /// The source stopped (bus closed, window destroyed). Observation ends cleanly.
    fn ended(&mut self);
}

/// (new) Replaces the direct calls from main.rs to lifecycle::native.
/// Object-safe: the app holds it as `Arc<dyn LifecycleSource + Send + Sync>`.
pub trait LifecycleSource {
    /// May wait for the system: the app asks through `probe_within`.
    fn snapshot(&self) -> SessionSnapshot;
    /// Dropping the returned watch stops it.
    fn watch(&self, listener: Box<dyn LifecycleListener>) -> io::Result<Box<dyn Send>>;
}

/// (new) In-memory: answers its snapshot; `send` and `end` drive the held watch.
pub struct InMemoryLifecycle { /* … */ }
```

The source stamps each change with the clock and the zone as it arrives,
as `LifecycleMessages` does for window messages; the Linux source takes a
clock and a zone the same way. `watch` takes `&self` and the snapshot is
read after it: the app listens before it reads the state, so that a change
meanwhile waits for the startup state (`WindowsLifecyclePorts.Attach`).

## History

```rust
// bientot-platform/src/power_history.rs
/// (new) One fact about power in the past, already decoded.
pub struct PowerBoundary {
    pub kind: PowerBoundaryKind,
    pub at: Timestamp,
    /// Stable identity of the source record, for idempotent recording.
    pub source_key: String,
}
pub enum PowerBoundaryKind { Boot, Shutdown, Suspend, Resume }

pub struct PowerBoundaries {
    pub boundaries: Vec<PowerBoundary>,
    /// False when the source stopped at `record_limit` or lost records.
    pub complete: bool,
}

pub enum HistoryUnavailable {
    /// The source exists but this user may not read it.
    AccessDenied,
    /// The source does not exist here (no journal, sandbox).
    NoSource,
    /// A record of the source could not be decoded.
    Malformed,
    Failed(String),
}

/// (new) Replaces SystemEventLog as the seam; the Windows record parser stays
/// in bientot-windows behind it.
pub trait PowerBoundaryLog {
    /// Ledger origin of what this log records, e.g. "windows-power-history".
    fn origin(&self) -> &'static str;
    fn read(&self, since: Timestamp, record_limit: usize)
        -> Result<PowerBoundaries, HistoryUnavailable>;
}

// bientot-platform/src/session_history.rs
pub struct SessionBoundary {
    pub kind: SessionBoundaryKind,
    pub at: Timestamp,
    pub source_key: String,
}
pub enum SessionBoundaryKind { Logon, Logoff, Lock, Unlock }

pub struct SessionBoundaries {
    pub boundaries: Vec<SessionBoundary>,
    /// False when the source stopped at `record_limit` or lost records.
    pub complete: bool,
}

/// (new) Whether a silence in the log means « nothing happened ».
pub enum LockHistory {
    /// Every lock in the window is in the log (Windows).
    Recorded,
    /// The platform keeps no record of locks (Linux). The recovery then
    /// follows D-026.
    NotRecorded,
}

pub trait SessionBoundaryLog {
    fn origin(&self) -> &'static str;
    fn lock_history(&self) -> LockHistory;
    fn read(&self, since: Timestamp, record_limit: usize)
        -> Result<SessionBoundaries, HistoryUnavailable>;
}
```

## Notifications

```rust
// bientot-core/src/notifications.rs — (existing, stays in core)
pub trait NotificationSink {
    fn show(&self, notification: &Notification)
        -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

// bientot-platform/src/notifications.rs
/// (new) Asked once at start and when a show fails.
pub enum NotificationAvailability { Available, Unavailable }

/// (new) A click on a notification's body. Clicks that arrive before the
/// handler (the one that started the process included) reach it first, in
/// order. The delivery id is `None` when the system gives none.
pub type ClickHandler = Box<dyn Fn(Option<&str>) + Send + Sync>;
pub trait NotificationClicks {
    fn on_click(&self, handler: ClickHandler);
}

/// (new) The queue behind every backend's clicks, and their in-memory form.
pub struct QueuedClicks { /* … */ }
impl QueuedClicks { pub fn click(&self, delivery_id: Option<String>); }

/// (new) Shows a notification the app already wrote.
pub trait Notifier {
    fn show(&self, request: &NotificationRequest)
        -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
/// (new) In-memory: keeps what it was asked to show.
pub struct InMemoryNotifier { /* … */ }
```

The platform cannot hand the app a `NotificationSink`: the sink writes the
reminder in the language of the settings, which are read after
`platform::build`. The platform hands a `Notifier` instead, which shows a
`NotificationRequest` already written; the app's `notify::ToastSink`
writes the request in the settings' language and hands it over. The
Windows `Toasts` implement `Notifier` in `bientot-windows`. A refused
registration reaches the app as `platform::NotificationsRefused`, the
backend's own error type (`windows::core::Error` on Windows), because the
diagnostic log names that type.

## Start at sign-in

```rust
// bientot-platform/src/startup.rs
/// (existing)
pub enum StartupRegistrationState {
    Unavailable, Disabled, DisabledByUser, DisabledByPolicy,
    Enabled, EnabledByPolicy, RegistrationMismatch,
}

/// (existing; variant WindowsStartup renamed)
pub enum LaunchIntent { Manual, SignIn }

/// (existing, from bientot-app::settings::startup; one method added)
pub trait StartupPort: Send + Sync {
    fn state(&self) -> StartupRegistrationState;
    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState;
    fn other_copy_starts(&self) -> bool;
    /// (new) Whether the platform has a system page where the user manages
    /// startup apps, and how to open it. `None` on Linux.
    fn system_page(&self) -> Option<SystemPage> { None }
}
pub struct SystemPage { pub uri: &'static str }
```

`system_page` answers `None` by default, so only a platform that has the
page implements it: Windows answers `ms-settings:startupapps`
(`bientot_windows::startup::STARTUP_APPS`), and the in-memory ports of the
tests need no change.
The settings and the onboarding views hand that page to their form when they
open (`show_startup_page`). The form shows « Open Startup apps » only when it
has a page, and opens its `uri`; without one, the startup row keeps its status
text.

Linux never returns `DisabledByUser`, `DisabledByPolicy` or
`EnabledByPolicy`.

## Single instance

```rust
// bientot-platform/src/single_instance.rs
/// (new; the Windows `Instance` enum becomes `Acquired<PrimaryInstance>`)
pub enum Acquired<L> {
    /// This launch holds the folder.
    Primary(L),
    /// Another process holds it and accepted this launch's request to show itself.
    Secondary,
}

pub trait InstanceLock: Sized {
    /// Holds the folder and receives the later launches until dropped.
    type Listener;
    fn acquire(directory: &DataDirectory) -> io::Result<Acquired<Self>>;
    /// Until then a later launch waits for its answer.
    fn listen(self, on_activation: impl FnMut() + Send + 'static)
        -> io::Result<Self::Listener>;
}

/// (new) In-memory: holds folders within this process only.
pub struct InMemoryInstance;
```

Acquiring and listening stay two steps, as on Windows today: the launch
claims the old folder before it moves it, and listens only once GPUI runs.
A later launch hands over nothing but its request; the instance key comes
from `DataDirectory::instance_key`.

## Updates

```rust
// bientot-platform/src/updates.rs — (existing, renamed from UpdateStore)
pub trait UpdateSource {
    type Offer;
    type Error;
    fn find_update(&self) -> impl Future<Output = Result<Option<Self::Offer>, Self::Error>>;
}
pub trait UpdateOffer {
    type Error;
    const RESTARTS: bool = false;
    fn download_and_install(&self) -> impl Future<Output = Result<(), Self::Error>>;
}
/// (existing) UpdateCheckSettings, UpdateService<…>
```

The Flatpak build has no `UpdateSource`: the settings group is hidden, as
it is for the portable ZIP.

## Tray

```rust
// bientot-platform/src/tray.rs
/// (existing, from bientot-app::tray_model) Generic over the command an item
/// runs, which the app defines: `tray_model::MenuEntry` is `MenuEntry<MenuCommand>`.
pub enum MenuEntry<C> {
    Item { label: String, command: C, enabled: bool },
    Submenu { label: String, items: Vec<MenuEntry<C>> },
    Separator,
}
/// (existing)
pub fn item_id(generation: u64, path: &[usize]) -> String;
/// (existing)
pub fn chosen<C: Clone>(entries: &[MenuEntry<C>], generation: u64, id: &str) -> Option<C>;

pub struct TrayIcon<'a> {
    /// Straight RGBA, one buffer per size, as tray_art::rgba draws them.
    pub pixmaps: &'a [(u32 /* px */, &'a [u8])],
}

pub struct TrayState<'a, C> {
    pub icon: TrayIcon<'a>,
    pub tooltip: &'a str,
    pub menu: &'a [MenuEntry<C>],
    pub generation: u64,
}

pub enum TrayHost { Present, Absent }

pub struct ScreenPoint { pub x: i32, pub y: i32 }

pub enum TrayEvent {
    /// Primary click. `at` is `None` when the host gave no usable position.
    Activated { at: Option<ScreenPoint> },
    MenuChosen { id: String },
    HostChanged(TrayHost),
}

pub trait Tray<C> {
    /// Sizes the host wants, in pixels, most wanted first.
    fn icon_sizes(&self) -> Vec<u32>;
    /// Whether the panel is dark, when the platform can tell.
    fn panel_is_dark(&self) -> Option<bool>;
    fn host(&self) -> TrayHost;
    fn publish(&self, state: TrayState<'_, C>);
}

/// (new) In-memory: answers what it was given, keeps what it was last given.
pub struct InMemoryTray<C> { /* … */ }

// crates/bientot-app/src/platform/mod.rs — bientot-platform takes no futures dependency
/// Built by the composition root with the channel events arrive on.
pub type TrayEvents = futures::channel::mpsc::UnboundedReceiver<TrayEvent>;
```

## Flyout placement

```rust
// bientot-platform/src/placement.rs — plain numbers, no GPUI type
pub struct Rect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }
pub enum Corner { TopLeft, TopRight, BottomLeft, BottomRight }

/// The part of a monitor no taskbar or panel covers.
pub struct WorkArea {
    pub display: u64,   // the system's id of the monitor, GPUI's `DisplayId`
    pub bounds: Rect,   // physical pixels
    pub scale: f32,     // physical pixels per logical pixel on that monitor
}

pub enum FlyoutPlacement {
    /// The app chooses the bounds, in a corner of the work area (Windows, X11).
    At { work_area: WorkArea },
    /// A layer-shell surface anchored to a corner (KDE, wlroots).
    Layer { corner: Corner, margin: i32 },
    /// The compositor chooses (GNOME on Wayland).
    CompositorPlaced,
    /// An ordinary, minimizable window (no tray host).
    Window,
}

pub trait Desktop {
    fn flyout_placement(&self, click: Option<ScreenPoint>, tray: TrayHost) -> FlyoutPlacement;
    /// Height of the area the flyout opens in, in physical pixels: the app
    /// keeps the panel 12 px from both edges, then scrolls its body.
    fn usable_height(&self, click: Option<ScreenPoint>) -> Option<i32>;
    /// Whether the system rounds and shadows the window (DWM) or the app draws both.
    fn system_draws_frame(&self) -> bool;
}

/// (new) In-memory: the same answers whatever it is asked.
pub struct InMemoryDesktop { pub placement, pub usable_height, pub system_draws_frame }
```

`shell.rs` maps `FlyoutPlacement` to `WindowOptions` (`WindowKind::PopUp`,
`Normal`) without a condition. GPUI opens a window on a display by its id
and takes its bounds in logical pixels at that display's scale, so `At`
names the monitor and its scale rather than bounds. The corner maths
(`shell::flyout_bounds`, 12 px from the edges) stays in the app: it fits
the window again whenever its content or its scale changes, with the
window's own scale. A corner other than the bottom right adds a `Corner`
to `At` with T060. `WindowKind::LayerShell` exists only in a GPUI built
for Wayland; until the Linux backend answers `Layer`, `shell.rs` opens it
as it opens `CompositorPlaced`.

When `system_draws_frame` is true, the flyout paints the border's colour
behind its panel, where the system's rounding leaves pixels; without a
desktop, as in the screenshots, it does not.

GPUI resizes a window but cannot move it, and `bientot-platform` cannot
name a GPUI window: `platform::place_window(window, client)` moves the open
flyout so that its client area covers `client`, in physical pixels (on
Windows, `SetWindowPos` past the frame, with DWM's round corners and no
border).

## Motion, system, process

```rust
// bientot-platform/src/motion.rs
pub type OnChange = Box<dyn Fn() + Send + Sync>;
pub trait ReducedMotion {
    /// Stops telling the changes when dropped.
    type Watch;
    fn current(&self) -> bool;
    /// `on_change` only queues; the app reads `current` afterwards, since the
    /// Windows hook runs inside a window procedure.
    fn watch(&self, on_change: OnChange) -> io::Result<Self::Watch>;
}
/// In-memory: `set` changes it and tells the watchers.
pub struct InMemoryMotion { /* … */ }

// bientot-platform/src/system.rs
pub enum InstallKind { Store, Direct, Portable, Flatpak, AppImage, Development }

pub struct SystemDescription {
    pub os_name: String,           // "Windows 11 26100", "Ubuntu 24.04.1 LTS"
    pub architecture: String,
    pub desktop: Option<String>,   // XDG_CURRENT_DESKTOP; None on Windows
    pub session_type: Option<SessionType>,
}
pub enum SessionType { X11, Wayland }

pub trait SystemInfo {
    fn install_kind(&self) -> InstallKind;
    fn describe(&self) -> SystemDescription;
    /// Names of the backends in use, for the diagnostic export:
    /// ("idle", "ext-idle-notify-v1"), ("tray", "status-notifier"), …
    fn backends(&self) -> Vec<(&'static str, String)>;
}
/// In-memory: the answers it was given.
pub struct InMemorySystem { pub install_kind, pub description, pub backends }
```

## The value the app receives

```rust
// crates/bientot-app/src/platform/mod.rs
/// The folder is held and the update installed before `build` runs, and
/// the notifications register before any window: `run_on` calls it first.
pub struct PlatformContext<'a> {
    pub directory: &'a DataDirectory,   // its path and instance key
    pub launch: &'a Launch,             // demo, --data-dir, origin, sign-in marker
}

pub struct Platform {
    pub launch_intent: LaunchIntent,
    pub last_input: Box<dyn LastInputProbe + Send>,
    pub lifecycle: Arc<dyn LifecycleSource + Send + Sync>,
    pub power_log: Box<dyn PowerBoundaryLog + Send>,
    pub session_log: Box<dyn SessionBoundaryLog + Send>,
    /// None in a demo and where no notification shows; Err when the system refused it.
    pub notifier: Option<Result<Box<dyn Notifier>, NotificationsRefused>>,
    pub notification_clicks: Box<dyn NotificationClicks>,
    pub startup: Option<Arc<dyn StartupPort>>, // None in a demo
    pub tray: Rc<dyn Tray<MenuCommand>>,       // shows nothing before its first publish
    pub tray_events: TrayEvents,
    pub desktop: Rc<dyn Desktop>,
}

impl Platform {
    /// Ports that answer « unknown » or do nothing: tests, screenshots, other OSes.
    pub fn null() -> Self;
}

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform>;

/// What a launch needs before `build`, or before it holds its folder: not in `Platform`.
pub type Instance;                       // impl InstanceLock
pub type Motion;                         // impl ReducedMotion; the failure window follows it too
pub fn motion() -> Motion;
pub fn run_update_hooks() -> bool;       // Velopack's hooks, before anything else
pub fn install_pending_update();
pub fn local_app_data() -> anyhow::Result<PathBuf>;
/// Last resort when no window can open, such as the import that waits for the next start.
pub fn show_fatal(title: &str, body: &str);
/// Returns when `pid` has exited or `timeout` passed; whether it exited. Runs before the
/// folder is held, for the restart after an import.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool;
/// Moves the open flyout's window, see Flyout placement.
pub fn place_window(window: &mut gpui::Window, client: Bounds<i32>);

/// The backend's error when the system refuses the notifications, see Notifications.
pub type NotificationsRefused;
/// The save and open dialogs; `None` once cancelled. Windows runs its blocking dialog on a
/// thread of its own; elsewhere GPUI's dialogs.
pub async fn pick_save(suggested: String, file_type: String, extension: &'static str,
    cx: &mut AsyncApp) -> anyhow::Result<Option<PathBuf>>;
pub async fn pick_open(file_type: String, extensions: &'static [&'static str],
    cx: &mut AsyncApp) -> anyhow::Result<Option<PathBuf>>;
/// The Store (`None` before the tray icon's window, which owns its prompts, exists, or
/// without package identity) and the Velopack feed. Elsewhere `updates::NoUpdates`, never
/// returned.
pub fn store_updates() -> Result<Option<impl UpdateSource>, impl Display>;
pub fn direct_updates() -> Option<impl UpdateSource>;
/// The system's own name for its zone when it is not an IANA name: the Windows key.
pub fn system_zone_id() -> Option<String>;
/// What the diagnostic snapshot keeps of the system, as .NET wrote it: zeros and `None`
/// where the system does not tell.
pub fn operating_system_version() -> [u32; 4];
pub fn native_machine() -> Option<u16>;            // an IMAGE_FILE_MACHINE
/// The diagnostic export's checks (`NoFinalPathCheck` off Windows), and where a folder
/// really is for the file manager (`None`: where its path says).
pub fn final_paths() -> &'static (dyn FinalPaths + Sync);
pub fn final_directory(directory: &Path) -> Option<PathBuf>;
/// Opens the settings file in the editor of its type. Does nothing off Windows, as before.
pub fn open_in_editor(file: &Path, cx: &App);
```

The backend functions after `place_window` are no fields of `Platform`:
the export of a failed start runs without one, and none of them holds
state. `SystemInfo` stays unwired until T121 gives it a consumer: its
`SystemDescription` names the system in words, while the snapshot keeps
.NET's numeric version, so `diagnostics.rs` reads
`operating_system_version` and `native_machine` meanwhile.

`build` runs before GPUI does and before the day is known, so the Windows
tray makes its icon at the first `publish`. The app quits when its last
window closes only when `tray.host()` is `Absent`.

Updates, the single-instance lock and the motion are generic rather than
boxed (their traits have associated types); `platform` exports the concrete
types under neutral names (`platform::Updates`, `platform::Instance`,
`platform::Motion`). `wait_for_exit` and `show_fatal` need system calls, so
they live in the backends rather than in `bientot-platform`.
