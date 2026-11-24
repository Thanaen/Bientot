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

/// (new) Receives changes on the source's own thread.
pub trait LifecycleListener: Send + 'static {
    fn changed(&mut self, change: LifecycleChange);
    /// The source stopped (bus closed, window destroyed). Observation ends cleanly.
    fn ended(&mut self);
}

/// (new) Replaces the direct calls from main.rs to lifecycle::native.
pub trait LifecycleSource {
    /// Dropping it stops the watch.
    type Watch: Send;
    fn snapshot(&self) -> SessionSnapshot;
    fn watch(self, listener: Box<dyn LifecycleListener>) -> io::Result<Self::Watch>;
}
```

`LifecycleEventRecorder` (existing) implements `LifecycleListener` by
stamping each change with the clock and the zone, as `LifecycleMessages`
does today for window messages.

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

/// (new) A click on a notification's body, delivered while the app runs.
pub trait NotificationClicks {
    fn on_click(&self, handler: Box<dyn Fn(/* delivery_id */ &str) + Send + Sync>);
}
```

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
    fn system_page(&self) -> Option<SystemPage>;
}
pub struct SystemPage { pub uri: &'static str }
```

Linux never returns `DisabledByUser`, `DisabledByPolicy` or
`EnabledByPolicy`.

## Single instance

```rust
// bientot-platform/src/single_instance.rs
/// (existing) What a second launch hands to the first.
pub struct Activation { /* open_flyout, intent, … as today */ }

/// (new)
pub enum Acquired<L> {
    /// This process is the instance; `L` listens until dropped.
    First(L),
    /// Another process took the activation.
    HandedOver,
}

pub trait InstanceLock: Sized {
    fn acquire(
        key: &InstanceKey,               // from bientot-store, per data folder
        activation: &Activation,
        on_activation: Box<dyn Fn(Activation) + Send + Sync>,
    ) -> io::Result<Acquired<Self>>;
}
```

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
/// (existing, from bientot-app::tray_model) MenuEntry, item_id, chosen

pub struct TrayIcon<'a> {
    /// Straight RGBA, one buffer per size, as tray_art::rgba draws them.
    pub pixmaps: &'a [(u32 /* px */, &'a [u8])],
}

pub struct TrayState<'a> {
    pub icon: TrayIcon<'a>,
    pub tooltip: &'a str,
    pub menu: &'a [MenuEntry],
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

pub trait Tray {
    /// Sizes the host wants, in pixels, most wanted first.
    fn icon_sizes(&self) -> Vec<u32>;
    /// Whether the panel is dark, when the platform can tell.
    fn panel_is_dark(&self) -> Option<bool>;
    fn host(&self) -> TrayHost;
    fn publish(&self, state: TrayState<'_>);
}

/// Built by the composition root with the channel events arrive on.
pub type TrayEvents = futures::channel::mpsc::UnboundedReceiver<TrayEvent>;
```

## Flyout placement

```rust
// bientot-platform/src/placement.rs — plain integers, no GPUI type
pub struct Rect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }
pub enum Corner { TopLeft, TopRight, BottomLeft, BottomRight }

pub enum FlyoutPlacement {
    /// The app chooses the bounds (Windows, X11).
    At { bounds: Rect },
    /// A layer-shell surface anchored to a corner (KDE, wlroots).
    Layer { corner: Corner, margin: i32 },
    /// The compositor chooses (GNOME on Wayland).
    CompositorPlaced,
    /// An ordinary, minimizable window (no tray host).
    Window,
}

pub trait Desktop {
    fn flyout_placement(&self, content: (i32, i32), click: Option<ScreenPoint>,
                        tray: TrayHost) -> FlyoutPlacement;
    /// Height the panel may use, to cap it and scroll its body.
    fn usable_height(&self, click: Option<ScreenPoint>) -> Option<i32>;
    /// Whether the system rounds and shadows the window (DWM) or the app draws both.
    fn system_draws_frame(&self) -> bool;
}
```

`shell.rs` maps `FlyoutPlacement` to `WindowOptions` (`WindowKind::PopUp`,
`LayerShell`, `Normal`); that mapping is the only GPUI-facing code and is
not conditional.

## Motion, system, process

```rust
// bientot-platform/src/motion.rs
pub trait ReducedMotion {
    fn current(&self) -> bool;
    fn watch(&self, on_change: Box<dyn Fn(bool) + Send + Sync>);
}

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
    /// Last resort when no window can open.
    fn show_fatal(&self, title: &str, body: &str);
}

/// Returns when `pid` has exited or `timeout` passed; whether it exited.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool;
```

## The value the app receives

```rust
// crates/bientot-app/src/platform/mod.rs
pub struct PlatformContext<'a> {
    pub data_dir: &'a Path,
    pub instance_key: &'a InstanceKey,
    pub launch: &'a Launch,
    pub demo: bool,
}

pub struct Platform {
    pub last_input: Box<dyn LastInputProbe + Send>,
    pub lifecycle: Box<dyn DynLifecycleSource>,       // object-safe wrapper
    pub power_log: Box<dyn PowerBoundaryLog + Send>,
    pub session_log: Box<dyn SessionBoundaryLog + Send>,
    pub notifications: Arc<dyn NotificationSink + Send + Sync>,
    pub notification_clicks: Box<dyn NotificationClicks>,
    pub startup: Arc<dyn StartupPort>,
    pub tray: Rc<dyn Tray>,
    pub tray_events: TrayEvents,
    pub desktop: Rc<dyn Desktop>,
    pub motion: Box<dyn ReducedMotion>,
    pub system: Arc<dyn SystemInfo + Send + Sync>,
    pub final_paths: Box<dyn FinalPaths + Send + Sync>,
}

impl Platform {
    /// Ports that answer « unknown » or do nothing: tests, screenshots, other OSes.
    pub fn null() -> Self;
}

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform>;
```

Updates and the single-instance lock are generic rather than boxed (their
traits have associated types); `platform` exports the concrete types under
neutral aliases (`platform::Updates`, `platform::Instance`).
