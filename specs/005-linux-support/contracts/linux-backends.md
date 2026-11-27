# Contract: Linux backends

What `bientot-linux` talks to, in which order it chooses, and what each
answer means. Reasons are in `research.md` (R4 to R11). Names marked (A-n)
rest on an assumption that a task verifies first.

## Session detection (once at start, again when a bus or compositor returns)

| Fact | Read from |
| --- | --- |
| Session type | `WAYLAND_DISPLAY` set → Wayland; else `DISPLAY` set → X11; else none |
| Desktop | `XDG_CURRENT_DESKTOP`, split on `:`; lower-cased; kept for diagnostics and for the default flyout corner |
| Install kind | `/.flatpak-info` present or `FLATPAK_ID` set → Flatpak; else `APPIMAGE` set → AppImage; else development (`bientot_linux::system::install_kind`) |
| Distribution | os-release's `ID` (`linux` when missing) and `VERSION_ID`, nothing else: `/run/host/os-release` first in a Flatpak, whose own `/etc` names the runtime, then `/etc/os-release`, then `/usr/lib/os-release`; kept for diagnostics |
| Own logind session | `GetSessionByPID(own pid)`, except in a Flatpak; else `XDG_SESSION_ID`; else the user's `Display` session |

The diagnostic export (`snapshot.json`, `system`) names the install kind,
the distribution, its version, the desktops, the session type and the
backend in use for the tray (`status-notifier` or `none`), the
notifications (`notification-server`, `notification-portal`, or `none`
when refused or in a demo), the autostart (`xdg-autostart`, or `none` in
a demo and, until T112, in a Flatpak), the idle time (the backend in use,
under the names below, or `none` when none answers and in a demo; absent
until the first selection ends)
and the lifecycle (`logind`, or `none` in a demo and once its watch did
not start), then `lockReporting`: `live`, or `unavailable` when the desktop
does not report screen locks or the lifecycle watch did not start; absent
in a demo, which starts no watch. Each name the system gives is
written only when it is 1 to 64 characters of `a-z`, `0-9`, `.`, `_` and
`-`, as os-release allows in `ID`
(`bientot_core::diagnostics::DiagnosticName`): free text, which could name
the user or the computer, never reaches the archive. « À propos » names
a Flatpak or an AppImage under the version. « Diagnostic », under it, names
the idle backend in use under « Méthode de détection » (« Aucune » for
`none`, with « Rien n’est compté automatiquement. ») and, once lock
reporting is known, whether locks are « Signalé » or « Non signalé », the
latter with `LockStateUnknown.Text`; both follow the backends' notes while
the window is open. Windows shows no such group.

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
  `tray.host` at each `HostChanged`. The export's projection of the log
  keeps only its fixed list of events, which has neither; the export names
  the tray's backend in `system.backends` instead (T121).
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
| Wayland 2 | `org.gnome.Mutter.IdleMonitor`, path `/org/gnome/Mutter/IdleMonitor/Core`, method `GetIdletime` → `t` (ms), session bus | The call answers | Polled |
| Wayland 3 | `org.freedesktop.ScreenSaver`, path `/org/freedesktop/ScreenSaver` or `/ScreenSaver`, `GetSessionIdleTime` | Its name has an owner (it is not started for this), it answers, and it passes the unit check: 11 reads 100 ms apart, over 1 s. A count that goes down between two reads fails the check, since input reset it. Otherwise the count is in milliseconds when it grew by the elapsed milliseconds ± 250 from the first read to the last, and in seconds when it grew by the elapsed seconds ± 1, at least 1, and never by more than 1 between two reads. Anything else fails the check | Polled |
| X11 1 | `MIT-SCREEN-SAVER` `QueryInfo` on the root window (`x11rb`) | The extension is present | `ms_since_user_input`, polled |
| X11 2 | Mutter IdleMonitor, then `org.cinnamon.Muffin.IdleMonitor` (same shape) | The call answers | Polled |
| — | none | — | `read()` returns `None` |

The selection runs at start, again at once when the backend in use stops
answering (its bus or compositor went away), and every 60 s while none
answers. Logged once: `idle.backend` (`name`: the backend, or `none` when
none answers); on change: `idle.backend_changed` (`from`, `to`), and
`idle.inhibitor_blind` (`value`: false) after the selection of ext-idle's
version 1. A line that leaves no backend is a warning. Names:
`ext-idle-notify-v1`, `org.gnome.Mutter.IdleMonitor`,
`org.cinnamon.Muffin.IdleMonitor`, `org.freedesktop.ScreenSaver`,
`MIT-SCREEN-SAVER`. The probe is made when the app starts, and a demo makes
none: as on Windows, a demo does not read the input. While the last note
names no backend, the flyout shows `ActivityUndetectable.*` in place of the
built-in source's failure, whose refresh and settings would change nothing,
with « Déclarer du temps », which opens « Déclarer du temps travaillé » on
today as the week does, and without the promise that the forecast comes back;
a demo's `none` comes from no note and shows the scenario's day.

The app reads the probe through `IdleWatch`, whose thread makes the probe,
then reads it every second and keeps the monotonic instant of the last
input. The UI thread's `read()` takes the idle time from that instant at its
own instant, so it never waits on the selection, a bus or a display server;
input shows within a second. Until the first selection ends, `read()`
returns `None`. Dropping the watch ends the thread.

`bientot_linux::idle`: `order` and `select` are the table (any OS, tested),
`IdleProbe` the `LastInputProbe`, `IdleWatch` the probe off the UI thread,
`SystemBackends` the backends (Linux).

## Lock, sleep, session: logind (system bus)

`bientot_linux::logind`: `SessionTracker` is the mapping (any OS, tested),
`LogindSource` the `LifecycleSource` (Linux). A demo watches no session, as
on Windows. A Flatpak skips `GetSessionByPID` and starts from
`XDG_SESSION_ID`, since its own pid namespace gives the process an id that
names another process on the host (`SessionLookup::for_install`). The export
names the source in `system.backends`. When its watch does not start (no
system bus, no logind, no session, or a Flatpak without the permission), the
diagnostic log gets the warning `lifecycle.watch_failed` (`kind`:
`not_found`, `permission_denied` or `other`; never the error's text) and the
export names `none`.

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
- The watch tells its listener the `LockReporting` of its own readings when
  it starts, then each change (`LifecycleListener::lock_reporting`): a
  screensaver that starts after the app, or `LockedHint` first seen true,
  turns it to `Live`. The app logs `lifecycle.lock_reporting` (`value`:
  `live`, or `unavailable` as a warning) and names it in the export; the
  snapshot, read apart from the watch, is not used for it.
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

`bientot_linux::notifications`: `notify`, `portal_notification` and
`Delivered` are the mapping (any OS, tested), `DesktopNotifications` the
`Notifier` (Linux).

- One session-bus connection of Bientôt's own, on a thread of its own,
  sends every notification and hears every click: servers that address
  their signals (KDE) send them to the connection that called `Notify`.
  `show` hands the request to that thread and returns; the UI thread never
  waits on D-Bus. Calls time out after 5 s.
- Unsandboxed: `org.freedesktop.Notifications.Notify` with
  `app_name = "Bientôt"`, `replaces_id = 0`, icon `dev.bientot.Bientot`,
  summary, body, actions `["default", ""]`, hint
  `desktop-entry = "dev.bientot.Bientot"`, `expire_timeout = -1`. The match
  rules for `ActionInvoked` and `NotificationClosed` are added before the
  first `Notify`. `ActionInvoked(id, "default")` for an id this process was
  given → click with the `delivery_id` kept for that `id`; other ids are
  other applications' notifications. `NotificationClosed(id)` forgets the
  id. `GetServerInformation` failing at start → `Unavailable`. The call is
  made directly over `zbus` rather than through `notify-rust`, which opens a
  connection per notification and keeps it until that notification is
  clicked or closed (GNOME keeps reminders in its list without closing
  them), and adds its match rule only after `Notify` returns.
- Flatpak (`InstallKind::Flatpak`):
  `org.freedesktop.portal.Notification.AddNotification` with id =
  `delivery_id`, `title`, `body` and `default-action = "open"`;
  `ActionInvoked(id, "open", _)` → click. Not
  `app.open`: the portal's backends hand an `app.…` action to the
  application's `org.freedesktop.Application.ActivateAction`, which Bientôt
  does not serve, and send `ActionInvoked` only for other names. The
  portal's `version` property unreadable at start → `Unavailable`. Over
  `zbus` too: through `ashpd`'s `notification` feature these two calls
  added 213 504 bytes to the stripped executable, over `zbus` 28 928.
- `Unavailable` is `platform::NotificationsRefused`: the app reserves no
  reminder (`notification.registration_failed` in the log). A `Notify` or
  `AddNotification` refused later leaves its reminder reserved, as a toast
  that fails on Windows; it is logged with `log`, not in the diagnostic log.
- A demo shows none, as on Windows.
- The last resort, `platform::show_fatal` when no window can open:
  `show_once` sends `last_resort(title, body)`, with delivery id
  `last-resort`, on a connection of its own, and returns once the service
  answered or a call timed out; the process then exits. A demo shows it
  too, as Windows shows its message box. Read in gnome-shell
  `js/ui/notificationDaemon.js`, plasma-workspace `libnotificationmanager`
  and `applets/notifications`, xdg-desktop-portal 1.22.1
  `src/notification.c` and xdg-desktop-portal-gnome `src/notification.c`
  (2026-11-27):
  - Server: `notify_once`, the `Notify` above without the `desktop-entry`
    hint. With it, GNOME Shell's `_getApp` finds the application, and
    `FdoNotificationDaemonSource._onNameVanished` destroys the source, and
    its notifications with it, once the sender leaves the bus; a click on
    one without the `default` action calls `open()`, which calls
    `app.activate()`. Without it, `_getApp` looks among the windows of the
    sender's process (`shell_window_tracker_get_app_from_pid`), which has
    none, then for `Bientôt.desktop`, which no application is named: the
    source has no application, `_onNameVanished` keeps it, and `open()`
    starts nothing. The source shows the `app_name` and the `app_icon`.
    Plasma removes no notification when its sender leaves (`server_p.cpp`
    watches only inhibitors and watchers), and its popup starts no
    application on a click (`Globals.qml`). It keeps the `default` action: Plasma takes no left click on a
    notification without one (`NotificationPopup.qml`), and closes it for a
    click with one (`Globals.qml`, `invokeDefaultAction`).
  - Portal: the `AddNotification` above, unchanged. When the application
    leaves the bus, the frontend only stops passing on the clicks on its
    notifications (`on_peer_disconnect`). On GNOME the backend
    hands it to `org.gtk.Notifications`, whose source watches no bus name
    and is kept across restarts of the shell. There a click on a
    notification without `default-action` calls the application's
    `activate()`, which would start Bientôt again
    (`GtkNotificationDaemonNotification.activate`); with `open`, the shell
    sends `ActionInvoked` and the backend calls
    `org.freedesktop.Application.Activate` on `dev.bientot.Bientot`, which
    starts Bientôt only through a D-Bus service file of that name: a
    package must not ship one. Plasma's backend (`portal_p.cpp`) keeps the
    notification and sends `ActionInvoked` for `open`.

## Start at sign-in

- Unsandboxed: file `$XDG_CONFIG_HOME/autostart/dev.bientot.Bientot.desktop`
  (`bientot_linux::autostart::XdgAutostart`; `$XDG_CONFIG_HOME` only when
  absolute, else `~/.config`):

  ```ini
  [Desktop Entry]
  Type=Application
  Name=Bientôt
  Exec="<absolute path of $APPIMAGE or of the executable>" --autostart
  Icon=dev.bientot.Bientot
  X-GNOME-Autostart-enabled=true
  ```

  The path is quoted by the `Exec` rules (`"`, `` ` ``, `$` and `\`
  escaped, `%` doubled), then written as a string value (`\` doubled).
  Until T114 the executable is `worktime`, so the entry names
  `…/worktime`; the file and its id do not depend on that name.
  States: file absent, or `Hidden=true`, or
  `X-GNOME-Autostart-enabled=false` → `Disabled`; `Exec`'s program is this
  install → `Enabled`; `Exec` is another path, has none or cannot be read →
  `RegistrationMismatch`; no absolute UTF-8 path for the executable, no
  configuration folder, or the file cannot be read or written →
  `Unavailable`. A `cargo` build in `target/` registers, as on Windows,
  where only development hosts are refused. Enabling writes the entry
  above (through `<file>.partial` and a rename) unless the state is already
  `Enabled`; disabling removes the file only when its program is this
  install. `other_copy_starts` is false: the Flatpak's portal writes its
  entry under the same file name, so the folder starts one copy at most,
  and the Flatpak's entry reads as `RegistrationMismatch`, which turning
  the switch on replaces. A demo gets no port, as on Windows.
- Flatpak (`InstallKind::Flatpak`, `bientot_linux::autostart::Autostart`):
  `org.freedesktop.portal.Background.RequestBackground` with
  `autostart = true|false`, `commandline = ["bientot", "--autostart"]`,
  `reason` from `contracts/copy.md`. The answer's `autostart` is stored in
  `app-state.json` (`autostartGranted`) and is the state shown (A-18).
- `--autostart` (`bientot_platform::startup::AUTOSTART_ARGUMENT`) means
  `LaunchIntent::SignIn` on every platform; Windows registrations keep
  writing `--worktime-windows-startup`, which still means the same.

## Single instance

- `bientot_linux::single_instance::PrimaryInstance`, `platform::Instance`
  on Linux. Directory `$XDG_RUNTIME_DIR/bientot/` (mode 0700;
  `$XDG_RUNTIME_DIR` only when absolute); fallback `<data folder>/run/`
  (mode 0700, the data folder itself created as usual).
- The files are named by the first 41 characters of the case-sensitive
  instance key (`WorkTime-` and 128 bits of the hash), the stem: a
  socket's path must fit the 108 bytes of `sun_path`, which the whole key
  under a data folder would not. Unlike Windows' key, it keeps the path's
  case, since `~/data` and `~/Data` are two folders on Linux.
- `<stem>.lock`: `flock(LOCK_EX | LOCK_NB)` (`File::try_lock`) held for
  the life of the first instance; never removed, so that two launches
  never lock two files of one name.
- `<stem>.sock`: stream socket, bound as the lock is taken, accepted on a
  thread of its own once the app listens. A second launch that fails the
  lock connects, writes the activation in the framing of the Windows pipe
  protocol (one byte; one byte back), waits for the acknowledgement
  (2 s), exits 0. Refused, missing or closed without an answer, it tries
  the lock again every 50 ms until then: an owner that left is replaced.
  Unanswered at 2 s, it exits as if accepted; its byte waits in the socket
  and the owner shows itself once it listens.
- A first instance that gets the lock removes a stale socket before
  binding. One that cannot bind (a path still too long) logs it and keeps
  the lock: later launches exit after 2 s without showing it.
- Dropping the listener wakes its thread with a connection of its own,
  joins it, removes the socket, then releases the lock.
- A restart for an import waits for the old process
  (`bientot_linux::process::wait_for_exit`) before it takes the lock: on a
  pidfd (`pidfd_open`, then `poll` for readable), which the kernel wakes
  once the process is a zombie or gone; where `pidfd_open` fails with
  anything but `ESRCH` (before Linux 5.3, or a seccomp filter that refuses
  it), `/proc/<pid>/stat` polled every 50 ms, a zombie counts as exited.

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
