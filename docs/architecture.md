# Architecture

Bientôt is one Rust process with a [GPUI](https://www.gpui.rs/) interface,
built for Windows and for Linux. The rules live in crates that build and test
on any OS; only the system calls and the windows on screen need the system
they are for. The Linux build runs but is not released, and it records no
activity ([Platforms](#platforms)). This page describes the code as of
November 2026.

## Crates

The Cargo workspace is `crates/*`. Each crate depends only on the crates above
it in this table.

| Crate | Builds on | Role |
| --- | --- | --- |
| `bientot-domain` | any OS | The accounting rules, as pure functions over a clock and a time zone: intervals, workday accounting, manual adjustments, weekly credit, forecasts and indicator state, daily summaries, reminders, settings validation, the usage-signal rules. |
| `bientot-store` | any OS (DPAPI on Windows) | Persistence in the data directory: `settings.json`, the SQLite ledger and its migrations, `app-state.json`, the DPAPI secrets, and the `.bientot` transfer file. |
| `bientot-core` | any OS | The app services: the activity source router, the accounting of one day, the weekly recalculation, tracking commands, corrections, long-lock reviews, reminders, the settings form logic, the demo scenarios, diagnostics, the usage signal sender. |
| `bientot-plugin-api` | any OS | The types Bientôt and its plugins exchange (api 1.0): the `plugin.json` manifest and its checks, the events, the day snapshot, the view tree of a card and its limits. |
| `bientot-plugins` | any OS | The plugin host: the package reader, signatures and trust, `plugins/state.json`, the WebAssembly runtime (wasmi) with its fuel, memory and time budgets, the host imports, the `plugins` thread, the plugin logs and the revocation list. |
| `bientot-plugin-sdk` | any OS, and `wasm32-unknown-unknown` | Writes plugins in Rust: the exports, the host calls, card builders and a native test host. Its `examples/` are the four sample plugins. |
| `bientot-plugin-tool` | any OS | The `bientot-plugin` command for plugin authors and the maintainer: `pack`, `keygen`, `sign`, `check`, `install`, `sign-revocations`. |
| `bientot-platform` | any OS | The platform-independent half of the system integration: the built-in activity source, the recording of session and power changes, the recovery of power and session history, the update check, the tray menu (`MenuEntry` and its item ids), the names of time zones on either system, `PowerState`. It defines the ports the app runs on ([Platforms](#platforms)). Those of the lifecycle, the single instance, the tray, the flyout placement, reduced motion, the languages, the system and the notifications have an in-memory implementation for tests. |
| `bientot-windows` | Windows for its Win32 calls | The last-input query, the session and power message decoding, power and session history from the System and Winlogon event logs, start with Windows (the `Run` key and the MSIX startup task behind `StartupPort`), toast notifications, the single-instance claim, file dialogs, the Microsoft Store `StoreContext` call. |
| `bientot-linux` | any OS; Linux for its system calls | The Linux integration (specs/005-linux-support): the tray icon as a StatusNotifierItem (`ksni`), where the flyout opens (X11, layer-shell on Wayland, or a window without a tray host), desktop notifications and the notification portal, the XDG autostart entry, the AppImage's entry in the menu of applications, the single instance, the install kind and the system the diagnostics describe, the language of the messages, the wait for a process to exit. Its idle readers (`idle`) give the app the time since the last input, and its logind source (`logind`) the lock, unlock, sleep and resume. `tests/support/` holds the D-Bus mocks of the desktop harness; `examples/shell-spike.rs` is the tray and flyout spike of Lot 4, never shipped. |
| `bientot-app` | Windows or Linux to run; tests on any OS | The `worktime` binary (`worktime.exe` on Windows): the composition root of the platform ports (`platform/`), the flyout, the week, « Corriger le temps… », settings, onboarding, the command palette, the theme and the FR/EN strings. It also holds the `bientot-screenshots` tool. |
| `bientot-notices` | any OS | Writes `THIRD-PARTY-NOTICES.txt` for the release executable and fails on a license outside the allowed list. |

The web service behind `bientot.dev` and `worktime.thanaen.dev` (website, usage
counter, update redirects, download link) is a separate Bun project in
`services/bientot`; see its
[README](../services/bientot/README.md).

## Platforms

The app reaches the system through ports: traits of `bientot-platform` that
`bientot-windows` and `bientot-linux` implement. The composition root is
`crates/bientot-app/src/platform/`. Its `mod.rs` picks one backend at compile
time, `windows.rs`, `linux.rs` or `null.rs` on any other system, and
re-exports what the backend gives:

- `build`, which makes the ports once per launch and returns them in
  `Platform`. `Platform::null()` gives ports that answer « unknown » or do
  nothing, for the tests and the screenshots.
- What a launch needs before it holds its data folder: `Instance`, `motion`
  and the folder that holds the data folder.
- Functions for the rest, such as the file dialogs, the update hooks and
  sources, the system's languages, the time zone's Windows key, the final
  paths and the message of last resort.

Outside its tests, the rest of `bientot-app` names no system, apart from the
layer-shell window of `shell.rs` and the `windows_subsystem` attribute of
`main.rs`. `linux.rs`
starts from the ports of `null.rs` and replaces those it has. `direct.rs`,
the Velopack updates, is compiled into both backends with the `updates`
feature. `platform::WORDING` makes a Linux build show the `.Linux` texts of
the string table ([CONTRIBUTING.md](../CONTRIBUTING.md#localization)).

| Port | Windows | Linux |
| --- | --- | --- |
| `LastInputProbe` | `GetLastInputInfo` | The first idle backend of the session that answers (`idle::IdleProbe`), read on a thread of its own (`idle::IdleWatch`); none in a demo |
| `LifecycleSource` | A message-only window that receives the session and power messages | The process's logind session on the system bus (`logind::LogindSource`); none in a demo |
| `PowerBoundaryLog`, `SessionBoundaryLog` | The System and Winlogon event logs | None |
| `Notifier`, `NotificationClicks` | Toasts | `org.freedesktop.Notifications`; the notification portal in a Flatpak |
| `StartupPort` | The `Run` key, or the MSIX startup task | An XDG autostart entry; none in a Flatpak |
| `LauncherPort` | None | The AppImage's entry in the menu of applications; none for other installs |
| `Tray` | The notification-area icon (`tray-icon`) | A StatusNotifierItem (`ksni`), with no bus name of its own in a Flatpak |
| `Desktop` | The corner of the taskbar's work area | A corner of the monitor's work area on X11, a layer-shell surface on Wayland, else where the compositor puts it; a window when no tray host shows the icon |
| `InstanceLock` | A named pipe | A lock and a Unix socket |
| `ReducedMotion` | « Animation effects » | None |
| `SystemLocale` | The display languages | `LC_ALL`, `LC_MESSAGES` or `LANG` |
| `SystemInfo` | None | The install kind, the session type, the desktops and os-release |

« None » is the null port of `Platform::null()`. `linux.rs` reads the install
kind once, from `/.flatpak-info`, `FLATPAK_ID` and `APPIMAGE`, and each
`bientot-linux` module picks its backend from it. Only the AppImage writes a
menu entry and updates through Velopack. The tray, notification, autostart,
idle and lifecycle backends in use go to the diagnostic export
(`platform::Backends`), on Linux only, with whether the desktop reports
screen locks. The idle backend chosen, and each later change of it, also go to
the diagnostic log (`idle.backend`), through `Platform::backend_notes`, as does
whether locks are reported (`lifecycle.lock_reporting`). Settings › Général ›
« Diagnostic » shows both, and the flyout says so while no idle backend
answers.

## Launch

`crates/bientot-app/src/main.rs` parses the command line (`launch.rs`),
resolves the data directory, then claims the single instance for that
directory (`platform::Instance`). If another copy of Bientôt already holds
it, the launch hands its activation over, logs `instance.redirected` and
exits before it opens the settings, the ledger or the log. A `--data-dir` or a
`--demo` launch uses another folder, so it runs on its own. The launch then
builds the ports once (`platform::build`).

On a first run, the onboarding window opens before anything else. Its first
step can import a `.bientot` or `.worktime` file instead (`transfer.rs`): Bientôt then
restarts, applies it before it loads the settings, and skips the onboarding,
since the file brings its settings. Otherwise
the app loads the settings and opens the day (`live.rs`): the diagnostic log,
the migrated ledger, the built-in source behind the source router, the weekly
recalculation and the tracking commands. When it cannot start, it opens a
window that says why and offers the diagnostic export
(`startup_failure_view.rs`).

## Data flow

1. **Activity.** While the built-in source is selected, `host.rs` samples the
   last keyboard or mouse input every 5 seconds. `bientot_platform::activity`
   turns the samples into active and idle spans with the configured idle
   threshold (180 s by default) and stores the settled ones. Time before
   Bientôt starts and the pending tail after the last input stay unknown. See
   the [built-in source contract](../specs/001-core-workday-indicator/contracts/built-in-activity-source.md).
   On Linux the probe answers « unknown », so the source reports that it
   cannot read input and records no span.
2. **Routing.** `bientot_core::routing` sends each part of a read to the
   source selected at that instant. ActivityWatch, an optional source of the
   first .NET app, is no longer read: the days it covered keep their stored
   summaries.
3. **Accounting.** Every 30 seconds, and on F5 or a tracking action,
   `bientot_core::accounting` computes the day from the spans, the session
   and power events, the manual adjustments and the recurring appointments.
   `bientot_core::weekly` then recalculates the current week, stores the
   daily summaries and the weekly credit, and closes earlier weeks.
4. **Display.** The tray icon, its tooltip, the flyout and the week read the
   published state (`workday.rs`, `tray_model.rs`, `week.rs`).
   `bientot_core::notifications` decides which reminders are due, and the
   notification receipts in the ledger make each one show once.

## Plugins

Plugins are built with bientot-app's `plugins` feature, which `release` and
`direct` turn on (spec 003). A plugin is a `.bientot-plugin` ZIP: a
`plugin.json` manifest, one WebAssembly module and its texts, signed by its
author or not. **Settings › Plugins** installs it after a consent dialog that
lists what it may do (`plugins.rs`, `settings_view/plugins.rs`).

- **Isolation.** Each module runs in [wasmi](https://github.com/wasmi-labs/wasmi),
  an interpreter, on a `plugins` thread of its own, never on the interface
  thread. Every plugin can log and read the time. It can read the published
  day as a snapshot, and set or clear its card, only when its manifest
  declares it and the user granted it. It cannot reach the network, the files
  or the ledger.
- **Budgets.** Every event has a fuel allowance and a safety net in time, and
  every instance a memory limit within a shared pool. A plugin that fails three
  times in ten minutes (a trap, its fuel used up or a malformed call) is
  stopped until the next start; the app never stops.
  Two starts in a row that crash with plugins on start the next one with them
  off (safe mode), and `--no-plugins` starts with none.
- **Display.** The host draws the card from a declarative tree the plugin
  sends, checked against fixed limits, and shows plugin commands in the
  command palette under « Plugins ». No plugin code draws anything.
- **Revocation.** A list signed with a key kept offline by the maintainer can
  turn a plugin or a signing key off, or every plugin, from a distance
  (`revocation.rs`). Release builds fetch it off the main thread with the
  daily update check and before an install or a turn-on
  (`bientot-app/src/plugins/revocations.rs`), and go by the last good list
  when it cannot be fetched.

With no plugin on, the start reads `plugins/state.json` if it exists and
starts no thread. The [plugin runtime specification](../specs/003-plugin-runtime/spec.md)
and its contracts give the details, and the
[SDK README](../crates/bientot-plugin-sdk/README.md) shows how to write one.

## Where state lives

The data directory is `%LOCALAPPDATA%\Bientot` on Windows and
`$XDG_DATA_HOME/Bientot` on Linux, which is `~/.local/share/Bientot` when
`XDG_DATA_HOME` is unset. A Flatpak's sandbox sets `XDG_DATA_HOME`, so a
Flatpak uses `~/.var/app/dev.bientot.Bientot/data/Bientot`. A
`--data-dir <path>` argument overrides it, and demo runs use a new folder
under `BientotDemo` in the temporary folder (`%TEMP%`, or `$TMPDIR` and else
`/tmp`). The first .NET app and the builds released before the rename to
Bientôt used `%LOCALAPPDATA%\WorkTime` with the same files, so the Rust app
reads their data with no conversion.

The first start after the rename moves that folder (`data_migration.rs`). When
only `WorkTime` exists, it is renamed `Bientot` in one step on the same
volume, before anything opens the database, so the folder is never split
between the two names. On Windows the start first claims the `WorkTime` folder:
an older build still running on it keeps it and shows its window, and nothing
moves. When the rename fails, that run stays on `WorkTime` and the next start
tries again. When both folders exist, `Bientot` is used and `WorkTime` is left
untouched. The log records `data_migration.moved`,
`data_migration.both_present` or `data_migration.failed`. A `--data-dir`
folder and a demo folder never move.

| Path | Content |
| --- | --- |
| `settings.json` | Schedule, notifications, recurring appointments, idle threshold, language, update check state, usage signal choice. Written through a temporary file and a rename. An invalid file is renamed to `settings.json.invalid-<guid>.json` and defaults are restored. |
| `worktime.db` | SQLite ledger: activity source selections, active and idle spans, session and power events, manual corrections, long-lock resolutions, daily summary revisions, credit ledger, notification receipts. |
| `app-state.json` | The date of the last usage signal the service recorded, and the flyout callouts closed today. |
| `secrets/*.secret` | Windows only: secrets encrypted with DPAPI for the current Windows user. |
| `logs/worktime-*.jsonl` | Structured diagnostic events, 5 files of 1 MiB at most by default, deleted after 14 days. |
| `plugins/` | With the `plugins` feature: one folder per installed plugin, `state.json` (versions, trust, grants, status), `starting` (the crashed-start counter) and `revocations.json` (the last good revocation list). |
| `logs/plugins/<id>.jsonl` | With the `plugins` feature: what each plugin logged and why the host stopped it, 1 MiB per file, deleted after 14 days. The diagnostic export does not read them. |
| `run/` | Linux only, when `XDG_RUNTIME_DIR` is unset: the lock and the socket of the single instance, which are otherwise under `$XDG_RUNTIME_DIR/bientot/`. |

[Privacy](privacy.md) lists what each file holds and every network request.

## Updates

- The build installed by `Setup.exe` (Velopack) checks
  `https://worktime.thanaen.dev/updates/` at most once a day, downloads an
  update in the background, and installs it at the next start or with
  « Redémarrer maintenant ».
- The Microsoft Store build asks the Store, at most once a day.
- The portable ZIP never checks; its settings link to the GitHub releases page.
- « Rechercher les mises à jour » in Settings turns the checks off.
- Only the `Setup.exe` build and the Linux AppImage carry Velopack: they are
  built with bientot-app's `direct` feature, the Store package and the ZIP
  with its `release` feature. A `direct` build on Linux checks only when it
  runs as an AppImage (`platform/direct.rs`, `platform/linux.rs`).

## Testability

The rules never reference GPUI or system types, so the domain, the store,
the services and the view models are tested on any OS. The portable halves of
`bientot-windows` and `bientot-linux` are tested on both systems, and their
system calls only on their own. The Linux calls run against the headless
desktop of `scripts/linux-desktop-harness.sh`: private D-Bus buses,
python-dbusmock's logind and notification server, mocks of a
StatusNotifierWatcher and of Mutter's idle monitor, and Xvfb with openbox or
sway without a screen. `crates/bientot-linux/tests/golden_activity.rs` and
`golden_ledger.rs` feed one day to the Windows and the Linux readers and
compare every row of the ledgers they write. `bientot-app` draws every demo
scenario off screen with `bientot-screenshots`, which CI uses to publish
screenshots of each pull request. See [CONTRIBUTING.md](../CONTRIBUTING.md).

## Related documents

- [Product specification](../specs/001-core-workday-indicator/spec.md)
- [Data model](../specs/001-core-workday-indicator/data-model.md)
- [Built-in activity source contract](../specs/001-core-workday-indicator/contracts/built-in-activity-source.md)
- [Windows power history recovery](power-history-recovery.md)
- [Start at sign-in](startup-registration.md)
- [Product decisions](product-decisions.md)
- [Plugin runtime specification](../specs/003-plugin-runtime/spec.md)
- [Privacy](privacy.md)
- [CI quality gates](ci-quality-gates.md)
- [Linux support specification](../specs/005-linux-support/spec.md) and its
  [Linux backends contract](../specs/005-linux-support/contracts/linux-backends.md)
