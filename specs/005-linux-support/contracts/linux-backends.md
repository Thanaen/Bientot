# Contract: Linux backends

What `bientot-linux` talks to, in which order it chooses, and what each
answer means. Reasons are in `research.md` (R4 to R11). Names marked (A-n)
rest on an assumption that a task verifies first.

## Session detection (once at start, again when a bus or compositor returns)

| Fact | Read from |
| --- | --- |
| Session type | `WAYLAND_DISPLAY` set → Wayland; else `DISPLAY` set → X11; else none |
| Desktop | `XDG_CURRENT_DESKTOP`, split on `:`; lower-cased; kept for diagnostics and for the default flyout corner |
| Install kind | `FLATPAK_ID` set → Flatpak; `APPIMAGE` set → AppImage; else development |
| Own logind session | `GetSessionByPID(own pid)`; else `XDG_SESSION_ID`; else the user's `Display` session |

## Tray: StatusNotifierItem (`ksni`)

`bientot_linux::tray`: `menu`, `pixmaps`, `click_position` and `HostWatch`
are the mapping (any OS, tested), `StatusNotifier` the `Tray` (Linux).

- Session bus, watcher `org.kde.StatusNotifierWatcher`. `ksni` runs with
  `assume_sni_available(true)`, so an item started before the watcher
  registers when it appears.
- Item: `Category = ApplicationStatus`, `Id = "dev.bientot.Bientot"`,
  `Title = "Bientôt"`, `IconPixmap` at 16, 22, 24, 32, 48 px (ARGB32,
  network byte order, converted from `tray_art::rgba`), `ToolTip` with the
  app's tooltip as its title and no description, `Menu` (dbusmenu) built
  from `MenuEntry`, `ItemIsMenu = false`. `Status = Passive` until the
  app's first `publish`, then `Active`: hosts hide a passive item.
  `publish` never waits on D-Bus: a thread hands `ksni` the state
  published last, and a state it has not handed yet gives way to the next.
- Menu labels double each `_`, which dbusmenu would read as an access key.
  The dbusmenu ids are `ksni`'s; each item carries its `item_id`.
- `Activate(x, y)` → `TrayEvent::Activated`; `at` is `None` when
  `x == 0 && y == 0`, the host's position otherwise, on Wayland too:
  waybar sends the pointer's position on the output (A-5, « Spike
  results »). Several outputs are untried.
- `SecondaryActivate` → ignored (the host shows the menu on a secondary
  click).
- dbusmenu `Event(clicked)` on item id → `TrayEvent::MenuChosen` with the
  id of `bientot_platform::tray::item_id`.
- The host is the watcher: KDE's and GNOME's watchers answer
  `IsStatusNotifierHostRegistered` with true whether a host runs or not,
  and `Error::WontShow` cannot come with `assume_sni_available(true)`.
  `watcher_online` → `HostChanged(Present)` at once; `watcher_offline`
  → `HostChanged(Absent)` once the watcher stayed away 5 s, nothing if it
  returns sooner. At start, `host()` is what `spawn` found, without a
  grace: `Absent` when no watcher owns its name.
- No session bus, or a watcher that refuses the item: no item, `host()`
  stays `Absent`, and the backend's name is `none`.
- Diagnostic log: `tray.backend` (`name`: `status-notifier` or `none`)
  and `tray.host` (`state`: `present` or `absent`) at start, then
  `tray.host` at each `HostChanged`. The export holds the log.
- In Flatpak: `disable_dbus_name(true)` (A-6).

## Flyout placement

`LinuxDesktop` (`bientot-linux/src/placement/native.rs`) takes the session
GPUI takes: Wayland when `WAYLAND_DISPLAY` is set, else X11 when `DISPLAY`
is, else none (`CompositorPlaced`). The maths is portable
(`placement.rs`) and tested on every system.

| Condition | `FlyoutPlacement` | How |
| --- | --- | --- |
| Tray host absent | `Window` | `WindowKind::Normal`, title « Bientôt », movable, minimizable. Focus loss and Esc minimize it instead of closing it; a second launch or `open_flyout` opens it again when it is not active. Closing it quits the app, whose last window it is: the quit mode follows the host (below) |
| X11 | `At { work_area, corner }` | Read at each opening: RandR `GetMonitors` (the root window when RandR names none) and `_NET_WORKAREA` of the current desktop. Monitor under the click, else the primary, else the first. Work area = that monitor ∩ `_NET_WORKAREA`, or the whole monitor when it misses it (A-2: with two monitors and a panel on one, the other comes out short by the panel). Corner of the work area nearest to the click; top right when the position is unknown. 12 px margin, which the app keeps. `display` is the X screen, GPUI's display id there; `scale` is GPUI's: `GPUI_X11_SCALE_FACTOR`, else `Xft.dpi` / 96, else 1 (GPUI's RandR fallback is left out; the first frame refits). `WindowKind::PopUp`, which GPUI opens override-redirect and cannot move: `platform::place_window` moves it with `ConfigureWindow`, and `platform::activate_window` gives it the input focus with `SetInputFocus`, which no window manager gives an override-redirect window (A-1). No RandR answer: `CompositorPlaced` |
| Wayland, `zwlr_layer_shell_v1` offered | `Layer { corner, margin: 12 }` | `WindowKind::LayerShell`, layer `Top`, keyboard `OnDemand` (A-3: sway gives it the focus only once the pointer enters it; `Exclusive` would have it at map, which T043 decides), namespace `bientot-flyout`, the margin on every edge. Hosts give the click on the output, not on the layout, so the corner is the nearest one on every output large enough to hold the click when they agree; else, or without a position, bottom right on `kde`, top right elsewhere. A layer surface that fails to open falls back to `CompositorPlaced` |
| Wayland otherwise | `CompositorPlaced` | `WindowKind::PopUp`, which GPUI's Wayland backend opens as an undecorated toplevel the compositor places (A-4, T044) |

`usable_height`: X11, the height of the work area read at the last
opening. Wayland, read once at start with `wl_output` (current mode,
transform) and `xdg_output` (logical position and size): the shortest
output's height in pixels, less a panel of 48 logical pixels that clients
cannot measure, since the compositor chooses the output. The app keeps the
panel 12 px from both edges, as on Windows.

GPUI 0.3.9 answers a layer surface's first `configure` with the size it
asked for before it, so a flyout whose content changed its height in its
first frame keeps the old size: `shell::lost_its_size` fits it again.

Window mode follows `HostChanged`: `shell::tray_changed` sets the quit
mode first (`Explicit` with a host, `Default` without), then without a
host opens the window when nothing shows and turns an open flyout into it;
with a host back, turns the window into the flyout when it has the focus
and closes it otherwise. A window replaces another only once open, so
that the app never runs without one. `windowModeNoticeDismissed` in
`app-state.json` is read and written; the notice itself is T064's.

## Idle

Each backend yields « idle for d » or nothing; `IdleProbe` turns it into a
`LastInputReading` with `reading_from_idle(CLOCK_MONOTONIC, d)`.

| Order | Backend | Probe that selects it | Reading |
| --- | --- | --- | --- |
| Wayland 1 | `ext_idle_notifier_v1` on a second Wayland connection | The global and a seat are advertised | `get_input_idle_notification` (version ≥ 2) with a 5 000 ms timeout, else `get_idle_notification` and `idle.inhibitor_blind=false` logged. `idled` at T means input stopped at T − 5 s; `resumed` means input now. Between events the idle time is computed from the last event: now − T + 5 s after `idled`; after `resumed` at R, now − R up to 5 s, since input went on within the timeout but not before R. The notification's creation counts as a `resumed` |
| Wayland 2 | `org.gnome.Mutter.IdleMonitor`, path `/org/gnome/Mutter/IdleMonitor/Core`, method `GetIdletime` → `t` (ms), session bus | The call answers | Polled every 5 s |
| Wayland 3 | `org.freedesktop.ScreenSaver`, path `/org/freedesktop/ScreenSaver` or `/ScreenSaver`, `GetSessionIdleTime` | Its name has an owner (it is not started for this), it answers, and it passes the unit check: 11 reads 100 ms apart, over 1 s. A count that goes down between two reads fails the check, since input reset it. Otherwise the count is in milliseconds when it grew by the elapsed milliseconds ± 250 from the first read to the last, and in seconds when it grew by the elapsed seconds ± 1, at least 1, and never by more than 1 between two reads. Anything else fails the check | Polled every 5 s |
| X11 1 | `MIT-SCREEN-SAVER` `QueryInfo` on the root window (`x11rb`) | The extension is present | `ms_since_user_input`, polled every 5 s |
| X11 2 | Mutter IdleMonitor, then `org.cinnamon.Muffin.IdleMonitor` (same shape) | The call answers | Polled every 5 s |
| — | none | — | `read()` returns `None` |

The selection runs at start, again at once when the backend in use stops
answering (its bus or compositor went away), and every 60 s while none
answers. Logged once: `idle.backend=<name>` (`none` when none answers); on
change: `idle.backend_changed`. Names: `ext-idle-notify-v1`,
`org.gnome.Mutter.IdleMonitor`, `org.cinnamon.Muffin.IdleMonitor`,
`org.freedesktop.ScreenSaver`, `MIT-SCREEN-SAVER`.

`bientot_linux::idle`: `order` and `select` are the table (any OS, tested),
`IdleProbe` the `LastInputProbe`, `SystemBackends` the backends (Linux).

## Lock, sleep, session: logind (system bus)

`bientot_linux::logind`: `SessionTracker` is the mapping (any OS, tested),
`LogindSource` the `LifecycleSource` (Linux).

- Service `org.freedesktop.login1`.
- Session object: `PropertiesChanged` on `org.freedesktop.login1.Session`
  for `LockedHint` and `Active`.
  - `LockedHint` true → `Session(Locked)`; false → `Session(Unlocked)`.
  - `Active` false → `Session(Locked)`; true → `Session(Unlocked)` unless
    the lock signal (below) says locked.
  - A change carrying both records one change; a property repeated, or a
    lock announced by two sources, records none.
- Manager: `PrepareForSleep(true)` → `Power(Suspended)`;
  `PrepareForSleep(false)` → `Power(Active)`, each once.
  `PrepareForShutdown` is not watched: logind sends `PrepareForShutdown(false)`
  after `PrepareForShutdown(true)` when the shutdown fails, and the session
  goes on.
- The system bus closing → `ended()`: the only end of the observation.
- The match rules are added before the snapshot is read, and the signals of
  one bus reach the mapping in the order they arrived.
- Snapshot at start: `LockedHint`, `Active` read once, and the screensaver's
  `GetActive` when its name has an owner (a screensaver that is not running
  is not started for it).
- Lock signal, which says whether the screen is locked:
  1. `LockedHint`, from the first time it is seen true (at start or later)
     **or** when the desktop is on the list that task T082 establishes (A-9;
     the list is empty until then);
  2. else `org.freedesktop.ScreenSaver` or `org.gnome.ScreenSaver` signal
     `ActiveChanged(b)` on the session bus → `Locked` / `Unlocked`, from the
     snapshot's `GetActive` or the first signal;
  3. else none: `LockReporting::Unavailable`, and the snapshot's session is
     `Unknown`. A switch of the session (`Active`) is still recorded.
- No inhibitor lock is taken. SIGTERM keeps its default action: the
  process ends, and every change already handed to the listener is in the
  ledger. Catching it in the source would delay a logout; whether the app
  quits cleanly on it, and so ends the activity observation, is the app's
  choice, not the source's.

## History

| Port | Source | Detail |
| --- | --- | --- |
| `PowerBoundaryLog`, journal | `journalctl --system --output=json --no-pager --since=<t>` filtered on `MESSAGE_ID` = `SD_MESSAGE_SLEEP_START`, `SD_MESSAGE_SLEEP_STOP`, plus `--list-boots --output=json` | `source_key` = `_BOOT_ID` + `__CURSOR`. Exit status or empty output with a permission hint → `AccessDenied`. Not installed, or Flatpak → `NoSource`. Origin `linux-power-history`. The exact message ids and the JSON fields are fixed by task T091 on real machines |
| `PowerBoundaryLog`, fallback | `/proc/stat` `btime`; `clock_gettime(CLOCK_BOOTTIME) − CLOCK_MONOTONIC` | One `Boot` boundary; `complete = false` when the difference exceeds 5 s (sleep happened, instants unknown). Origin `linux-boot-time` |
| `SessionBoundaryLog` | logind session property `Timestamp` (µs since the epoch) | One `Logon` boundary; `lock_history() = NotRecorded`. Origin `linux-session-history` |

## Notifications

- Unsandboxed: `org.freedesktop.Notifications.Notify` through
  `notify-rust`: `app_name = "Bientôt"`, summary, body, icon
  `dev.bientot.Bientot`, hint `desktop-entry = "dev.bientot.Bientot"`,
  action `default`. `ActionInvoked(id, "default")` → click with the
  `delivery_id` kept for that `id`. `GetServerInformation` failing →
  `Unavailable`.
- Flatpak: `org.freedesktop.portal.Notification.AddNotification` with id =
  `delivery_id`, `default-action = "app.open"`; `ActionInvoked` → click.

## Start at sign-in

- Unsandboxed: file `$XDG_CONFIG_HOME/autostart/dev.bientot.Bientot.desktop`:

  ```ini
  [Desktop Entry]
  Type=Application
  Name=Bientôt
  Exec="<absolute path of $APPIMAGE or of the executable>" --autostart
  Icon=dev.bientot.Bientot
  X-GNOME-Autostart-enabled=true
  ```

  States: file absent, or `Hidden=true`, or
  `X-GNOME-Autostart-enabled=false` → `Disabled`; `Exec` is this install →
  `Enabled`; `Exec` is another path → `RegistrationMismatch`; development
  build → `Unavailable`. `other_copy_starts` is true when the file of the
  other install kind's id exists (not visible from the sandbox: false).
- Flatpak: `org.freedesktop.portal.Background.RequestBackground` with
  `autostart = true|false`, `commandline = ["bientot", "--autostart"]`,
  `reason` from `contracts/copy.md`. The answer's `autostart` is stored in
  `app-state.json` (`autostartGranted`) and is the state shown (A-18).

## Single instance

- Directory `$XDG_RUNTIME_DIR/bientot/` (mode 0700); fallback
  `<data folder>/run/`.
- `<instance key>.lock`: `flock(LOCK_EX | LOCK_NB)` held for the life of
  the first instance.
- `<instance key>.sock`: stream socket. A second launch that fails the
  lock connects, writes the activation in the framing of the Windows pipe
  protocol, waits for the acknowledgement (2 s), exits 0.
- A first instance that gets the lock removes a stale socket before
  binding.

## Motion and theme

- `org.freedesktop.portal.Settings.Read`:
  `org.freedesktop.appearance` / `reduced-motion` (1 = reduce) when
  present; else `org.gnome.desktop.interface` / `enable-animations`
  (false = reduce). `SettingChanged` for both (A-15).
- Theme: GPUI already follows `color-scheme`.

## D-Bus names the Flatpak must be allowed to talk to

`org.kde.StatusNotifierWatcher`, `org.gnome.Mutter.IdleMonitor`,
`org.cinnamon.Muffin.IdleMonitor`, `org.freedesktop.ScreenSaver`,
`org.gnome.ScreenSaver` (session); `org.freedesktop.login1` (system).
Portals need no declaration.
