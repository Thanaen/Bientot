# Research: Linux support

Done on 24 November 2026 on `main` at `d9f55ce`, on Linux x86-64 (Ubuntu
24.04, glibc 2.39) with toolchain 1.97.0.

Every statement below carries one of these marks:

- **Code**: read in this repository or in the source of a pinned dependency.
- **Run**: observed by running a command here; the command is given.
- **Source**: read on the page cited in « Sources » at the end.
- **Assumption A-n**: not verified. The task that verifies it is named, and
  nothing in the plan may rely on it before that task is done.

## R1. What a Linux build does today

**Run.** `cargo check --workspace --all-targets --locked` passes on Linux in
1 min 12 s with no warning. `cargo check --locked -p bientot-app --bin
worktime --features direct` passes too. This is not news: the `Rust` job of
`ci.yml` already lints and tests the whole workspace on Ubuntu with
`--all-targets --all-features` (**Code**).

It proves less than it seems. `velopack`, `tray-icon`, `raw-window-handle` and
`windows` are `cfg(windows)` dependencies, and `updates::direct` and `tray`
are `cfg(windows)` modules, so the `direct` build on Linux contains no update
code and no tray (**Code**: `crates/bientot-app/Cargo.toml`, `lib.rs:32`,
`updates.rs:9`).

**Run.** `cargo build --release --locked -p bientot-app --bin worktime
--features release` takes 5 min 04 s and gives a stripped executable of
15 537 912 bytes (15.54 MB), against 15.4 MB for `worktime.exe`
(`Cargo.toml`). `ldd` lists `libssl.so.3`, `libcrypto.so.3`, `libgcc_s`,
`libm` and `libc`; the highest symbol version is `GLIBC_2.39`.

**Run.** Started under `dbus-run-session xvfb-run` on a throwaway
`--data-dir` with `{}` as `settings.json` and `--open-flyout`, the process
stays alive, creates `worktime.db`, writes `BIENTOT_READY_FILE`, logs
`startup.tray_ready` and uses 15.8 MB of resident memory. It shows no tray
icon and no window (the root window stays blank), writes nothing to standard
error, and records no activity. This container has no Vulkan driver, so
whether the flyout would draw with one is **Assumption A-13** (task T031).

So a Linux user who built Bientôt today would get a silent process that
counts nothing: `main.rs` substitutes `NoLastInput`, no lifecycle is watched,
no notification sink is registered and `tray::install` is never called
(**Code**: `main.rs:40-52`, `244-245`, `457-458`).

## R2. Inventory of Windows-specific code

Difficulty: **S** under a day, **M** a few days, **L** a week or more, for
the Linux equivalent, tests included.

### Crate `bientot-windows` (9 452 lines, of which about 2 400 are Win32)

Each module is a portable half, tested on any OS, and a `native` half behind
`cfg(windows)` (**Code**).

| Module | What it does | Why Windows-specific | Linux equivalent | Difficulty |
| --- | --- | --- | --- | --- |
| `activity.rs` | `BuiltInActivitySource`: samples a `LastInputProbe` every 5 s, settles active and idle spans, stores them | Nothing: portable | Reused as is from `bientot-platform` | S |
| `activity/native.rs` | `SystemLastInput`: `GetLastInputInfo`, `GetTickCount64` | Win32 | One probe per backend: ext-idle-notify-v1, Mutter IdleMonitor, `org.freedesktop.ScreenSaver`, XScreenSaver (R6) | L |
| `lifecycle.rs` | Decodes `WM_WTSSESSION_CHANGE` and `WM_POWERBROADCAST`; `LifecycleEventRecorder` writes transitions to the ledger | Decoding is Win32-shaped; the recorder is portable | Recorder reused; logind signals produce the same `LifecycleTransition` (R7) | M |
| `lifecycle/native.rs` | Message-only window on its own thread, `WTSRegisterSessionNotification`, `current_session_state` | Win32 | A D-Bus thread on the system bus | M |
| `power_history.rs` | Pairs shutdown/boot and suspend/resume boundaries, records them idempotently | Pairing and recording are portable; event ids and XML are Windows | Pairing and recording reused; boundaries from the journal (R8) | M |
| `power_history/native.rs`, `event_log/` | `EvtQuery` on the System log, XML rendering | Win32 Event Log | `journalctl --output=json` filtered by message id | M |
| `session_history.rs` | Winlogon event 811 to logon, logoff, lock and unlock boundaries; « Avant Bientôt » recovery | Recovery is portable; the source is Winlogon | logind and the journal give the session opening and sleep, not locks (R8) | M |
| `session_history/native.rs` | Query of `Microsoft-Windows-Winlogon/Operational`, user SID | Win32 | logind `Session.Timestamp`, journal | M |
| `startup.rs`, `startup/native.rs`, `registry.rs` | `Run` key value, `StartupApproved` state, legacy migration | Registry | XDG autostart `.desktop` file; Background portal in Flatpak (R10) | M |
| `startup/packaged.rs` | MSIX `StartupTask`, activation kind | WinRT | Background portal (Flatpak) | M |
| `single_instance.rs`, `single_instance/native.rs` | Named pipe per data folder and session; hand-over of the activation | Named pipes, session id | Unix socket under `$XDG_RUNTIME_DIR`, same protocol (R11) | M |
| `notifications.rs`, `notifications/native*`, `notifications/portable*` | WinRT toasts, AppUserModelID registration, COM activator that restarts the app on a click | WinRT, COM, registry | `org.freedesktop.Notifications` through `notify-rust`; portal in Flatpak (R9) | M |
| `updates.rs` | `StoreUpdateService`: at most one check a day, behind the setting | Nothing: portable | Reused for the Velopack feed on Linux | S |
| `updates/native.rs` | Microsoft Store `StoreContext` | WinRT | None: Flathub updates outside the app (R13) | S |
| `animation_effects*` | Follows « Animation effects » through `SPI_GETCLIENTAREAANIMATION` and a message hook | Win32 | Settings portal (R12) | S |
| `diagnostics*` | Final paths under MSIX redirection, OS version, native machine | Win32 | `/etc/os-release`, `uname`; no path redirection to check outside Flatpak | S |
| `dialogs*` | Common item dialogs on a thread | COM | GPUI's portal file chooser, already wired off Windows (`file_dialogs.rs:25-60`) | S |
| `process*` | Waits for the process that restarted this one | Win32 handle | `pidfd_open` or polling `/proc/<pid>` | S |
| `lib.rs` `has_package_identity` | MSIX or not | WinRT | `FLATPAK_ID`, `APPIMAGE` environment (R13) | S |
| `lib.rs` `dotnet_ticks` | .NET ticks in operation keys | Nothing: portable | Reused | S |

### Crate `bientot-app`

| File | What it does | Why Windows-specific | Linux equivalent | Difficulty |
| --- | --- | --- | --- | --- |
| `main.rs` (40 `cfg` lines, `mod native` of 240 lines) | Wires lifecycle, history catch-up, toasts, single instance, animation setting, tray, message box | Calls `bientot-windows` directly | Same wiring through the ports (plan, « Composition root ») | M |
| `tray.rs` (369 lines, whole module `cfg(windows)`) | `tray-icon` and `muda` menu, icon size from DPI, taskbar theme from the registry, clicked monitor, flyout placement with `SetWindowPos`, DWM corners | Win32, registry, DWM | StatusNotifierItem through `ksni` (R4); placement per session type (R5) | L |
| `shell.rs:134-152, 238-255` | Opening bounds, `fit_window`, work area height | Calls `tray.rs` | Placement port | M |
| `host.rs:634` | `tray::show` at each publish | `cfg(windows)` | Port call | S |
| `notify.rs:44-60` | `ToastSink` | Windows toasts | A sink over the notifier port | S |
| `updates.rs`, `updates/direct.rs` | Store and Velopack sources; hooks at startup; `Setup.exe` restart | WinRT; Velopack compiled for Windows only | Velopack on Linux for the AppImage (R13) | M |
| `file_dialogs.rs` | Thread around the Windows dialogs | COM dialogs block | Already uses GPUI off Windows | S |
| `zones.rs:28-47` | Local zone id from `GetDynamicTimeZoneInformation` (a Windows key name such as `Romance Standard Time`) | Ledger compatibility with .NET | IANA id on Linux already; a Windows ledger imported on Linux carries Windows names (R12) | M |
| `diagnostics.rs:182-224` | Final paths and OS version of the snapshot | Win32 | `/etc/os-release` | S |
| `transfer.rs:200-205` | Waits for the restarting process | Win32 | See `process` | S |
| `plugins.rs:945` | Real path of the plugins folder under MSIX | MSIX redirection | Not needed; `cx.open_with_system` works | S |
| `settings/startup.rs`, `settings_view.rs` | `StartupPort` over `bientot_windows::startup` types; « Open Startup apps » (`ms-settings:startupapps`) | Types and wording | Same port, Linux states (R10) | M |
| `launch.rs` | `--worktime-windows-startup`, `--toast-activated`, `-Embedding` | Run key marker, COM activation | `--autostart`; no COM (R10) | S |
| `build.rs`, `build/version_resource.rs` | Version resource through `embed-resource` | PE resource; already skipped off Windows | Desktop entry and metainfo carry name and version | S |
| `strings/*.resw` (17 strings name Windows in each language) | « Démarrage Windows », « Langue de Windows », « Les notifications Windows… », « Votre session Windows… » | Wording | Platform variants (R12) | M |
| `tray_art.rs`, `tray_model.rs`, `flyout*`, `week*`, `settings*`, `palette*`, `adjust*`, `onboarding*`, `widgets.rs`, `theme.rs`, `assets.rs` | Everything the user sees | Nothing: GPUI, bundled IBM Plex fonts and Lucide icons | Reused | none |

### Other crates

| File | What it does | Why Windows-specific | Linux equivalent | Difficulty |
| --- | --- | --- | --- | --- |
| `bientot-store/src/secrets/dpapi.rs` | DPAPI secret store | Win32 | None needed: no secret is written since ActivityWatch is no longer read (`docs/architecture.md`). Out of scope | none |
| `bientot-store/src/data_migration.rs` | `WorkTime` to `Bientot` folder move | Legacy of Windows installs | Never applies: no Linux build ever used `WorkTime` | S |
| `bientot-core/src/diagnostics/export.rs:767-791` | Directory and reparse-point tests | File attributes | Already has a Unix branch | none |
| `bientot-plugins`, `bientot-plugin-api`, `bientot-plugin-sdk`, `bientot-plugin-tool` | Plugin host and tools | Nothing (R14) | Reused | none |
| `bientot-plugins/examples/hostile-corpus.rs:484-540` | Working set of the process | Per-OS call | Already has a Linux branch | none |
| `bientot-domain` | Rules | Nothing; `LastInputReading` is shaped like `GetTickCount` | Reused; Linux probes build the same reading | none |

### Packaging, scripts, CI, service, documents

| Item | What it does | Why Windows-specific | Linux equivalent | Difficulty |
| --- | --- | --- | --- | --- |
| `packaging/msix/`, `packaging/store/` | MSIX manifest, Store listing | Microsoft Store | `packaging/linux/`: desktop entry, AppStream metainfo, icons, Flatpak manifest | M |
| `scripts/package-rust.ps1`, `verify-package-*.ps1`, `verify-startup-visibility.ps1`, `test-rust-toast-activation.ps1`, `measure-performance.ps1` | Package, verify and measure on Windows | PowerShell, MSIX, Win32 | Bash scripts for Linux (`measure-performance.sh`, `package-linux.sh`) | M |
| `scripts/release-direct.sh` | Velopack `win` pack, `.ico`, cross-build with `cargo-xwin`, `vpk download --channel win` | Target and channel are fixed | A target argument; `vpk pack` for Linux makes the AppImage | M |
| `scripts/release-notes.sh`, `check-release-summary.sh`, `store-release-notes.sh` | Notes and user summaries | Not specific, but one list for every reader | A platform mark per line (R15) | S |
| `.github/workflows/ci.yml` `rust-windows` | Tests, release build, 20 MB gate, startup and memory, MSIX install, Velopack build | `windows-latest` | A `Rust on Linux` job with the same gates (R15) | M |
| `.github/workflows/release.yml` | Builds on `windows-latest`, publishes Windows files, attests them | One platform | Second build job, same release (R15) | M |
| `.github/workflows/store-package.yml`, `store-listing.yml` | Microsoft Store submission | Store | Flathub pull request workflow (R13) | M |
| `rust-toolchain.toml` | Targets `x86_64-pc-windows-msvc`, `wasm32-unknown-unknown` | List | Add nothing: the host target is installed by default; `aarch64-unknown-linux-gnu` builds on its own runner | none |
| `services/bientot/src/releases.ts` | `/download`, `/updates/` redirect to `Bientot-Setup.exe`, `releases.win.json`, `*.nupkg` | File names | Add the Linux names (R16) | S |
| `services/bientot/src/ping.ts`, `store.ts`, `stats.ts` | Accepts exactly `version`, `channel`, `first_this_month`; channels `store`, `direct`, `zip` | Closed lists | New channels and a platform field (R16) | M |
| `services/bientot/site/*.html` | « pour Windows », FAQ « Mac ou Linux ? Non », JSON-LD `operatingSystem` | Wording | Platform-aware download (R16) | M |
| `README.md`, `CONTRIBUTING.md`, `docs/*.md` | « a small Windows app », « Windows 11 x64 », Windows-only procedures | Wording | Rewritten per platform (tasks T180–T183) | M |
| `.specify/memory/constitution.md` | Principle III « Windows system tray », Principle V « Windows and .NET platform libraries », « Initial platform support is limited to… Windows » | Written for one platform | Amendment 1.3.0 (plan) | S |
| `.specify/scripts/powershell/` | Spec Kit scripts | Tooling choice (`"script": "ps"`) | Not touched: `pwsh` runs on Linux | none |

Not Windows-specific, checked: fonts (IBM Plex Sans and Mono are compiled in,
`assets.rs`); icons (Lucide SVG, compiled in); the data folder (already
`$XDG_DATA_HOME` off Windows, `main.rs:490-498`); the demo folder
(`std::env::temp_dir()`).

## R3. GPUI on Linux at the pinned revision

`gpui-pre` 0.3.9 is a snapshot of `zed@93d7456`; its Linux platform is
`gpui-pre-linux` 0.3.9, read in `~/.cargo/registry` (**Code** for all of
this section).

- **Backends.** Features `wayland` and `x11`, both on by default, plus a
  headless platform. Rendering goes through `gpui-pre-wgpu`; text through its
  `font-kit` feature. The CI screenshots already run on it with Mesa's
  software Vulkan (`ci.yml`, `gpui-screenshots`).
- **Window kinds.** `WindowKind` has `Normal`, `PopUp`, `AnchoredPopup`,
  `Floating`, `Dialog` and, on Linux with Wayland, `LayerShell`.
- **X11.** `PopUp` creates an override-redirect window of type
  `_NET_WM_WINDOW_TYPE_NOTIFICATION` at the requested bounds
  (`x11/window.rs:514`, `616`). `AnchoredPopup` is refused
  (`PopupNotSupportedError`, `x11/window.rs:479`). The scale factor comes
  from `Xft.dpi` (`x11/client.rs:2672`): one scale for every monitor. GPUI
  does not read `_NET_WORKAREA`.
- **Wayland.** `PopUp` is an ordinary `xdg_toplevel`: the compositor places
  it and the requested origin is ignored (`wayland/window.rs`, « All other
  WindowKinds result in a regular xdg surface »). `LayerShell` creates a
  `zwlr_layer_surface_v1` with layer, anchor, margins and keyboard
  interactivity, and fails with `LayerShellNotSupportedError` when the
  compositor lacks the protocol (`wayland/window.rs:161`). `AnchoredPopup`
  is an `xdg_popup` and needs a parent window of the app
  (`wayland/window.rs:207`), so it cannot be anchored to a tray icon, which
  belongs to the shell. Fractional scaling is bound
  (`wp_fractional_scale_manager_v1`), with `wp_viewporter` and
  `xdg_activation_v1`.
- **No tray.** Nothing in `gpui-pre` or `gpui-pre-linux` mentions a tray or
  StatusNotifierItem.
- **No idle.** GPUI binds neither `ext_idle_notifier_v1` nor the XScreenSaver
  extension (`x11rb` is built without its `screensaver` feature).
- **Notifications.** `Platform::show_system_notification` exists and uses
  `notify-rust` 4.18 over `org.freedesktop.Notifications`
  (`system_notifications.rs`), with a `default` action for a click on the
  body. It cannot close a notification.
- **Portals.** `ashpd` 0.13 is built with `file_chooser`, `open_uri`,
  `settings` and `inhibit`; `open_with_system`, `open_url` and the file
  prompts use them.
- **Accessibility.** `accesskit_unix` is a dependency of both backends.
  Whether Orca reads the flyout as Narrator does is **Assumption A-16**
  (task T127).
- **Already in `Cargo.lock`**, so free to use: `zbus` 5.19, `ashpd` 0.13.13,
  `notify-rust` 4.18.2, `wayland-client`, `wayland-protocols` (with
  `staging`, which holds `ext-idle-notify-v1`), `x11rb`, `oo7`, `rustls`
  0.23.

Not verified, to settle in the spike (Lot 4):

- **A-1**: on X11 an override-redirect `PopUp` receives the keyboard focus
  after `activate_window` and reports its deactivation, so the flyout closes
  on a click outside. Window managers do not manage such windows. If it
  fails, the flyout becomes an undecorated `Normal` window with
  `_NET_WM_STATE_SKIP_TASKBAR` and `_NET_WM_STATE_ABOVE`, set through a
  second `x11rb` connection. Task T041.
- **A-3**: a `LayerShell` surface with `KeyboardInteractivity::OnDemand`
  gets focus and reports its loss on KWin, Sway and Hyprland. Task T043.
- **A-4**: on GNOME Wayland an undecorated `xdg_toplevel` of the flyout's
  size is placed acceptably by Mutter. Task T044.
- **A-13**: what GPUI draws on a machine without a Vulkan driver. Task T031.

## R4. Tray

GNOME Shell has no tray of its own; KDE Plasma, Cinnamon, Xfce, MATE, LXQt,
Budgie, COSMIC and the bars of wlroots compositors implement the
StatusNotifierItem (SNI) host over D-Bus. Ubuntu ships the AppIndicator
extension, which is such a host; Fedora Workstation and Debian's GNOME do
not.

| Option | Finding | Outcome |
| --- | --- | --- |
| `tray-icon` 0.26 with its default `libappindicator` backend | **Source**: needs GTK 3 and libappindicator at run time, and cannot report left clicks or tooltips (tauri#11293). A GTK main loop beside GPUI's | Rejected |
| `tray-icon` 0.26.1 with its `ksni` feature | **Code** (crate source): pure D-Bus, keeps the `muda` menu code of `tray.rs`. But it calls `assume_sni_available(true)` and hides `watcher_online`/`watcher_offline`, so the app cannot know that no host exists; it reports `SecondaryActivate` as a middle click | Rejected: the fallback of FR-012 needs the host state |
| **`ksni` 0.3 directly** | **Code**: `Tray::activate(x, y)`, `secondary_activate`, `menu()`, `icon_pixmap()`, `tool_tip()`, `watcher_online`, `watcher_offline(reason)`, `Error::WontShow` when no host exists, `disable_dbus_name` for sandboxes. Uses `zbus`, already in the lock file | **Chosen** |
| XEmbed tray (legacy X11) | Absent on Wayland, dropped by GNOME | Rejected |
| A window instead of a tray everywhere | Breaks Principle III | Rejected; it is the fallback only |

**Decision.** `ksni` behind the tray port, in `bientot-linux`. The menu
comes from `tray_model::MenuEntry`, the pixels from `tray_art::rgba`, as on
Windows. Windows keeps `tray-icon`.

- **Icon.** SNI takes ARGB pixmaps of several sizes; Bientôt sends 16, 22,
  24, 32 and 48 px and lets the host choose. The panel's colour is unknown
  (on Windows the registry tells the taskbar theme), so the Linux icon needs
  an outline that reads on light and dark panels: a design task (T053, UI).
- **Left click.** `Activate` opens the flyout. Hosts that only show a menu
  (some GNOME extensions when `ItemIsMenu` is true) get « Ouvrir Bientôt »
  as the first menu entry, which already exists.
- **Click position.** `Activate(x, y)` gives screen coordinates on X11
  hosts; on Wayland hosts it is often `0, 0`. **Assumption A-5**, task T041.
- **GNOME without the extension, or no host.** `ksni` reports
  `WontShow` or `watcher_offline`. Bientôt then runs in **window mode**
  (R5): the flyout is an ordinary window that minimizes instead of closing
  on focus loss, a later launch from the app grid brings it back (the
  single-instance hand-over that exists today), and the flyout shows once
  a notice that names the missing tray and links to the help page. When a
  host appears (`watcher_online`), the icon shows and window mode ends
  without a restart.
- **Flatpak.** **Source** (Flatpak desktop integration): SNI needs
  `--talk-name=org.kde.StatusNotifierWatcher`; owning
  `org.kde.StatusNotifierItem-PID-ID` is not required by current hosts, and
  `ksni`'s `disable_dbus_name(true)` avoids it. **Assumption A-6**: this
  works on the Tier 1 desktops. Task T131.

## R5. Flyout placement

Windows places the flyout in the corner of the work area of the clicked
monitor (`shell::flyout_bounds`). Linux has three cases.

| Session | Mechanism | What the user sees |
| --- | --- | --- |
| X11 | `PopUp` (or the fallback of A-1) at computed bounds. Work area from `_NET_WORKAREA` and the monitor under the click from RandR, read through `x11rb` by `bientot-linux`. The panel may be at the top (GNOME, Cinnamon option) or bottom: the flyout goes to the corner of the work area nearest to the click, top-right when the click position is unknown | Same as Windows, at the panel's edge |
| Wayland with `zwlr_layer_shell_v1` (KDE Plasma, Sway, Hyprland, COSMIC, niri, labwc) | `LayerShell`, layer `Top`, anchored to two edges with the 12 px margin, on the output GPUI is given. The corner is the one nearest to the click when the host gave a position, else a per-desktop default (KDE: bottom-right; others: top-right) | Same as Windows, in a screen corner. It may not be the corner where the icon is when the panel was moved; a setting is out of scope |
| Wayland without layer-shell (GNOME) | `xdg_toplevel` without decorations. No client can choose its position on Wayland, and `xdg_popup` needs a parent surface of the app (R3) | **Different**: the flyout opens where Mutter puts new windows, usually the centre of the current monitor, with the same content, size and rounded panel. It closes on focus loss, Esc or a second click on the icon, as elsewhere |
| No tray host (window mode, R4) | `Normal` window with the app's title bar handling, minimizable | **Different**: a small ordinary window titled « Bientôt » |

`work_area_height` (used to cap the panel and scroll its body) comes from
the same source: `_NET_WORKAREA` on X11, the output's logical height minus
margins on Wayland, where the usable area is not known to clients.

Rounded corners and shadow: on Windows DWM rounds and shadows the window. On
Linux the window is transparent around the panel, which draws its own
radius and shadow (`WindowBackgroundAppearance::Transparent`); X11 without a
compositing manager shows square corners. Design task T064 (UI).

Options rejected: asking the user for a corner in the settings (a setting
for something every other OS does alone; can come later if A-5 fails
everywhere); a GNOME Shell extension shipped by Bientôt (a second product to
maintain against every GNOME release); XWayland for positioning (blurry
with fractional scaling, and Mutter still places XWayland windows).

## R6. Idle detection

Windows asks `GetLastInputInfo` every 5 s. The contract is the existing
`LastInputProbe`: « how long since the last keyboard or pointer input, or
`None` when it cannot be told ». `BuiltInActivitySource` and the domain's
`IdleStateTracker` stay untouched; each Linux backend builds a
`LastInputReading` from a monotonic uptime and an idle duration.

| Backend | Where it works | Notes |
| --- | --- | --- |
| `ext-idle-notify-v1` over a second Wayland connection | **Source** (wayland.app): KWin 6.7, Sway 1.11, Hyprland, COSMIC, niri, labwc at version 2. **Not Mutter** (51), not Weston | Event-based: one notification with a 5 s timeout; `idled` and `resumed` give the idle start within the sampling error the Windows probe already has. Version 2's `get_input_idle_notification` ignores idle inhibitors; version 1 does not, so a video call that inhibits idle would look active. Version 2 is required for parity; with version 1 only, the backend is used and the difference is logged and documented. **Assumption A-7**: Tier 1 KWin offers version 2. Task T072 |
| `org.gnome.Mutter.IdleMonitor` (`GetIdletime` on `/org/gnome/Mutter/IdleMonitor/Core`, session bus) | GNOME on Wayland and X11, Cinnamon's Muffin exposes `org.cinnamon.Muffin.IdleMonitor` | **Source**: idle time in milliseconds, input-based. Polled every 5 s like Windows |
| `org.freedesktop.ScreenSaver.GetSessionIdleTime` (session bus) | KDE, some others | Units differ between implementations; used only when the two above are absent, after a sanity check against a known idle start |
| XScreenSaver extension (`x11rb` feature `screensaver`, `QueryInfo`) | X11 sessions | Milliseconds, input-based. Under XWayland it only sees input sent to X clients, so it is never used in a Wayland session |

**Selection at run time**, once at start and again when the session bus or
compositor restarts. The first that answers a probe wins:

1. Wayland session (`WAYLAND_DISPLAY` set): ext-idle-notify-v1, then Mutter
   IdleMonitor, then `org.freedesktop.ScreenSaver`.
2. X11 session: XScreenSaver, then Mutter or Muffin IdleMonitor.
3. None: the probe returns `None`. The built-in source then reports itself
   unhealthy as it does on Windows when the query fails, the flyout says
   that activity cannot be detected on this desktop, and nothing is counted
   automatically (Principle II). Manual declarations still work.

The chosen backend is written to the diagnostic log (`idle.backend`) and
shown in the diagnostic export.

Rejected: reading `/dev/input` (needs the `input` group, reads every device:
Principle I); logind's `IdleHint` (set by the desktop after its own timeout,
minutes late, and not at all by several desktops); the portal (no idle
interface exists; `Inhibit` only prevents idle).

Flatpak: the Wayland socket gives ext-idle-notify; Mutter IdleMonitor needs
`--talk-name=org.gnome.Mutter.IdleMonitor`. **Assumption A-8**: Flathub
accepts it with a justification. Task T136.

## R7. Lock, unlock, suspend, resume, session changes

**Source** (`org.freedesktop.login1`): the session object has the properties
`LockedHint`, `Active`, `State`, `Timestamp`, and the signals `Lock` and
`Unlock`; the manager has `PrepareForSleep(bool)` and
`PrepareForShutdown(bool)`. `Lock` and `Unlock` are **requests** sent to the
session's locker (after `loginctl lock-session`), not reports that the
screen locked. `LockedHint` is the report, set by the desktop through
`SetLockedHint`.

| Windows event | Linux source | Maps to |
| --- | --- | --- |
| `WTS_SESSION_LOCK` / `UNLOCK` | `PropertiesChanged` of `LockedHint` on the own session (system bus). Fallback when a desktop never sets it: `ActiveChanged` of `org.freedesktop.ScreenSaver` or `org.gnome.ScreenSaver` (session bus) | `LifecycleChange::Session(Locked / Unlocked)` |
| Switch User (disconnect, reconnect) | `Active` false / true on the own session | `Session(Locked / Unlocked)`, as Windows records a disconnect as a lock |
| `PBT_APMSUSPEND` / `PBT_APMRESUMEAUTOMATIC` | `PrepareForSleep(true / false)` | `Power(Suspended / Active)` |
| Session state at start (`WTSQuerySessionInformation`) | `LockedHint` and `Active` read once | `prepare_lifecycle`'s probe |
| Logoff | The process is terminated; `PrepareForShutdown(true)` and SIGTERM both end the observation cleanly | `end_observation` |

The own session is found with `GetSessionByPID`, or `XDG_SESSION_ID`, or the
user's `Display` session when the process runs in the user manager's scope
(the case of autostart on GNOME).

**Assumption A-9**: GNOME, KDE Plasma and Cinnamon keep `LockedHint` true
while locked; lockers such as `swaylock` and `i3lock` do not, and offer no
screensaver interface either. On such a desktop the session state is
`Unknown` and Bientôt says so; the day is then accounted from activity
alone, as on Windows when the lifecycle window cannot open. Task T082.

Without systemd-logind or elogind (some distributions), no source exists:
same degradation. No delay inhibitor is taken: Bientôt has nothing to finish
before sleep that the ledger does not already hold.

Flatpak: `--system-talk-name=org.freedesktop.login1`. **Source** (Flathub
linter): only `org.freedesktop.DBus` and wildcards are refused for the
system bus. **Assumption A-10**: this name passes review. Task T136.

## R8. History before Bientôt was running

Windows gives two histories without elevation: power (System log) at every
start, and session (Winlogon log) at the first start only, which rebuilds
« Avant Bientôt ».

What Linux offers without root:

| Fact | Source | Available |
| --- | --- | --- |
| Start of the current boot | `/proc/stat` `btime`; `CLOCK_BOOTTIME` | Always |
| Total time asleep since boot | `CLOCK_BOOTTIME − CLOCK_MONOTONIC` | Always; a total, not intervals |
| Opening of the current session | logind `Session.Timestamp` | With logind |
| Suspend and resume instants | Journal entries of `systemd-sleep` with the catalog ids `SD_MESSAGE_SLEEP_START` and `SD_MESSAGE_SLEEP_STOP`; previous boots through `_BOOT_ID` | Only when the user can read the system journal: members of `adm`, `wheel` or `systemd-journal`, which is the first user of Ubuntu, Fedora and Mint (**Assumption A-11**, task T091). Not in Flatpak |
| Shutdown and boot of earlier boots | Journal boot list (first and last entry of each `_BOOT_ID`) | Same condition |
| Earlier logons and logoffs | `wtmp` (`last`) | **Source**: being retired for Y2038; Debian 13 no longer ships `last`, several distributions moved to `wtmpdb`. Not used |
| Lock and unlock in the past | Nothing records them: `LockedHint` is a current value, and desktops do not log locks | **Never** |

**Decision.**

- **Power history** (every start): boundaries from the journal when it is
  readable, paired and recorded by the existing code; the ledger origin is
  `linux-power-history`. When it is not readable: the current boot's start
  is recorded as a boot boundary (which, as on Windows, clears old session
  knowledge without inventing powered-off time), and when
  `CLOCK_BOOTTIME − CLOCK_MONOTONIC` is not zero the log says that sleep
  happened at unknown instants. The time stays unknown and is offered for
  manual declaration, as after an unreadable System log on Windows.
- **Session history** (first start only): Linux can tell when the session
  opened and, with the journal, when the computer slept, but never when the
  screen was locked. The Windows rule refuses a history with a possible
  hidden lock (`docs/session-history-recovery.md`: « a missing record could
  hide a lock »). Applying it strictly gives nothing on Linux. This is an
  accounting question (constitution, workflow rule 2), so it is **open
  question Q4** for the owner. The plan implements the recommended answer
  behind the same review: rebuild from the session opening to Bientôt's
  start, minus sleep when known, as one « Avant Bientôt » review whose text
  says that locks are not known on Linux; without the journal, the same
  from the session opening only; inside Flatpak, the same from logind
  alone. The time is provisional and reviewed, never silently credited.
  Lot 9 does not start before Q4 is answered.

Flatpak: journal access would need `--filesystem=/var/log/journal:ro` and
host libraries; not requested. The Flatpak build has session opening from
logind and nothing about past sleep.

## R9. Notifications

| Option | Finding | Outcome |
| --- | --- | --- |
| GPUI `show_system_notification` | Exists (R3), but is tied to the GPUI thread and tags | Not used directly: the sink is called from the accounting refresh with a `delivery_id` |
| **`notify-rust`** behind `NotificationSink`, in `bientot-linux` | Already compiled in. `org.freedesktop.Notifications`: summary, body, icon name, `desktop-entry` hint, `default` action | **Chosen** outside a sandbox |
| **Notification portal** (`org.freedesktop.portal.Notification`) | The sandboxed path; `ashpd` has a `notification` feature | **Chosen** inside Flatpak |

Differences from Windows, all platform limits:

- **Click.** A click opens the flyout while Bientôt runs. On Windows a click
  on a toast left in the notification centre restarts Bientôt through COM.
  On Linux a server may start the app from the `desktop-entry` hint, or do
  nothing; Bientôt does not depend on it.
- **Actions.** Servers advertise `actions` in `GetCapabilities` or not
  (**Source**: the specification makes them optional). Bientôt's
  notifications carry no button today, only the body click, so nothing is
  lost. **Assumption A-17**: body clicks arrive on GNOME, KDE and Cinnamon,
  and through the portal. Task T101.
- **Refused or absent server.** The existing `notifications_refused` path
  applies: the flyout says notifications are unavailable, and no reminder is
  reserved in the ledger without being shown.

## R10. Start at sign-in

| Install | Mechanism | State the settings show |
| --- | --- | --- |
| AppImage, unpackaged | `~/.config/autostart/dev.bientot.Bientot.desktop` (XDG autostart), `Exec=<absolute path> --autostart`, `X-GNOME-Autostart-enabled`, `Hidden` | Enabled, Disabled, Registration mismatch (the file names another path: the AppImage was moved, as for a moved ZIP on Windows), Unavailable (a development build) |
| Flatpak | Background portal `RequestBackground` with `autostart: true` and the command line; the portal writes the host's autostart file | The sandbox cannot read the host file. The state is the portal's last answer, kept in `app-state.json`, asked again when the user flips the switch. **Assumption A-18**: asking does not prompt again on GNOME and KDE once granted. Task T112 |

`--autostart` is the neutral name of `--worktime-windows-startup`, which
Windows keeps writing and reading (installed copies depend on it,
`docs/startup-registration.md`). Both mean « start hidden ».

« Open Startup apps » has no Linux counterpart (each desktop has its own
tool) and is hidden. Policy states (`DisabledByPolicy`, `EnabledByPolicy`)
do not exist on Linux. A systemd user unit was rejected: it starts before
the graphical session is ready on several desktops, and Flatpak cannot
install one.

## R11. Single instance

The Windows design is a named pipe per data folder and session, which
vanishes with its owner (`single_instance.rs`). On Linux:

| Option | Finding | Outcome |
| --- | --- | --- |
| **Unix socket** `$XDG_RUNTIME_DIR/bientot/<instance key>.sock` plus an advisory lock (`flock`) on `<key>.lock` | Same protocol as the pipe (connect, send the activation, wait for the answer); the lock is released by the kernel when the owner dies, so a stale socket is detected and replaced; `$XDG_RUNTIME_DIR` is per user and, in Flatpak, per app; works without a session bus | **Chosen** |
| D-Bus well-known name | One name per data folder is impossible to declare ahead for a sandbox (`--own-name` wants fixed names); adds a dependency on the session bus for a `--data-dir` run in CI | Rejected |
| Lock file alone | Detects, but cannot hand the activation over | Rejected |

The instance key is the one `bientot-store` already derives from the data
folder (#189). A Flatpak copy and an AppImage copy have different data
folders (R12), so they are two instances, like two `--data-dir` runs.

## R12. Paths, fonts, locale, opening things, and the rest

- **Data folder.** One folder, as on Windows: `$XDG_DATA_HOME/bientot`
  (`~/.local/share/bientot`), holding `settings.json`, `worktime.db`,
  `app-state.json`, `logs/` and `plugins/`. The XDG split between config,
  state and cache is not followed: the instance key, `--data-dir`, the
  `.bientot` transfer, the diagnostic export and « Open folder » all assume
  one root, and splitting would add an abstraction with no gain for the
  user. Today's code gives `…/share/Bientot` (`launch.rs:417`); no Linux
  build was ever released, so the lower-case name costs nothing. In Flatpak
  the same code lands in `~/.var/app/dev.bientot.Bientot/data/bientot`.
  Velopack keeps its packages under `$XDG_CACHE_HOME` or `/var/tmp` on its
  own. **No migration** exists or is needed on Linux; `data_migration`
  stays a Windows step.
- **Cross-install data.** On Windows the Store build and `Setup.exe` share
  `%LOCALAPPDATA%\Bientot`. On Linux the sandbox separates them. The bridge
  is the existing export and import.
- **Time zones.** A `.bientot` file exported on Windows carries Windows zone
  key names in the ledger (`zones.rs:28`). `SystemZones::get` asks `jiff`'s
  database by name. **Assumption A-14**: `jiff` does not resolve Windows
  names on Linux, so an import from Windows needs the CLDR `windowsZones`
  mapping in `zones.rs`. Task T125 tests it with a real Windows export
  first.
- **Fonts.** Bundled; identical glyphs. Rasterization differs (DirectWrite
  on Windows, `font-kit` on Linux), which the screenshots of both platforms
  will show; no token changes.
- **Tray icon rendering.** `tray_art::rgba` already draws at any size with
  `resvg`; see R4 for the outline.
- **Locale.** `Language::resolve` does not read the system language on any
  platform yet (« Those are not read yet, so French stands for them »,
  `strings.rs:39-41`). Parity is kept by doing the same thing on both:
  reading `LC_MESSAGES`/`LANG` on Linux and the display languages on
  Windows belongs to one cross-platform task (T126), with spec 004's
  region default (its T040) on the same port. The 17 strings that name
  Windows get Linux variants (`contracts/copy.md`).
- **Opening URLs, files and folders.** `cx.open_url` and
  `cx.open_with_system` use the OpenURI portal or `xdg-open`. Nothing to
  write.
- **File dialogs.** `file_dialogs.rs` already calls GPUI's prompts off
  Windows; they use the file chooser portal.
- **Reduced motion.** GPUI's `set_reduce_motion` is fed on Linux from the
  Settings portal: `org.gnome.desktop.interface` `enable-animations`, and
  the `org.freedesktop.appearance` `reduced-motion` key where the portal
  has it. **Assumption A-15**: which key each Tier 1 desktop serves. Task
  T123.
- **Light and dark.** GPUI reads `color-scheme` from the Settings portal;
  `ThemeMode` follows it as on Windows.
- **Startup failure.** The window is GPUI's. The last-resort message box
  (`MessageBoxW`) becomes: standard error, plus a desktop notification when
  a server answers.
- **TLS.** `ureq` is built with `native-tls`, which is OpenSSL on Linux:
  the executable links `libssl.so.3` dynamically (R1). An AppImage would
  then depend on the host's OpenSSL major version. Decision: on Linux,
  `ureq` uses `rustls` with the platform verifier (system certificates),
  statically linked; Windows keeps SChannel. `rustls` 0.23 is already in
  the lock file. **Assumption A-20**: it costs under 1.5 MB. Task T037
  measures it before anything depends on it.

## R13. Packaging and distribution

What the app needs from a package: a tray (session bus), idle (Wayland
socket or session bus), logind (system bus), optionally the journal,
autostart, notifications, HTTPS, and updates.

| Channel | Tray | Idle | logind | Journal | Autostart | Updates | Cost to maintain | Outcome |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Flatpak on Flathub** | `--talk-name=org.kde.StatusNotifierWatcher` | Wayland socket; `--talk-name=org.gnome.Mutter.IdleMonitor`, `org.freedesktop.ScreenSaver` | `--system-talk-name=org.freedesktop.login1` | No | Background portal | Flathub and the software centre; the app never updates itself | One manifest, built by Flathub for x86_64 and aarch64 (**Source**), reviewed permissions | **Primary** |
| **AppImage with Velopack** | Direct | Direct | Direct | Yes when the user may read it | XDG autostart file | Self-update from the feed, like `Setup.exe` (**Source**, **Code**) | One `vpk pack` per architecture in the release workflow | **Secondary** |
| `.deb` / `.rpm` | Direct | Direct | Direct | Yes | XDG | A repository to host and sign for each family, or no updates | Two formats, several distribution versions | Not at launch |
| AUR | — | — | — | — | — | Through the AUR helper | Belongs to its maintainer; can wrap the AppImage or build from source | Community, not official |
| Snap | Classic confinement needed for the tray and logind without manual `snap connect`; classic needs Canonical's approval | Same | Same | No | Yes | Snap Store | A second store review for one distribution family | Rejected |
| Tarball | Direct | Direct | Direct | Yes | XDG | None | Small | Rejected: the AppImage already is a single portable file |

**Why Flathub first.** It is the Linux counterpart of the Microsoft Store:
one listing, reviewed, signed and updated by the store, present in the
software centre of Fedora, Mint, Pop!_OS and others, with x86_64 and
aarch64 builds from one manifest. Its sandbox costs one feature: journal
history (R8).

**Why the AppImage second.** It is the counterpart of `Setup.exe`: every
feature, nightly, self-updating. It serves users without Flatpak, and is the
build CI measures.

So the channels pair up: `store` ↔ `flatpak`, `direct` ↔ `appimage`. The
portable ZIP has no Linux twin.

**Velopack on Linux**, at the pinned 1.2.161. **Code** (crate source):
`DEFAULT_CHANNEL_NAME` is `linux`; the locator finds the app from
`current_exe()` under `…/usr/bin/`, requires `UpdateNix` beside it, and
replaces the file named by `$APPIMAGE`. **Source** (Velopack documentation):
`vpk pack` builds the AppImage from the build folder, needs a PNG icon,
takes `--categories` and `--compression`; an update downloads to `/var/tmp`
and replaces the AppImage, asking for `pkexec` only when the file sits in a
privileged folder; there is no installer and no desktop integration, so the
app writes its own launcher entry on first run (T144). The feed is
`releases.linux.json` beside `releases.win.json`.
**Assumption A-12**: the whole flow (check, download, replace, restart)
works for an AppImage in the user's home on the Tier 1 distributions, and
`velopack` builds for `aarch64-unknown-linux-gnu`. Task T140.

**Flatpak specifics.**

- App id `dev.bientot.Bientot` (the project owns `bientot.dev`, which
  Flathub's verification uses). An id cannot change after publication:
  **open question Q2**.
- Runtime `org.freedesktop.Platform`; no GTK or Qt is needed.
- **Source**: Flathub builds offline, so the manifest carries the vendored
  crate sources (`flatpak-cargo-generator`), regenerated when `Cargo.lock`
  changes (CI check, T133).
- Permissions: `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc`,
  `--device=dri`, `--share=network` (update of the revocation list and the
  usage signal), the D-Bus names above. No `--filesystem` beyond the
  portal: imports and exports go through the file chooser.
- The updater is compiled out (`release` features, like the Store build).
- Cadence: a Flathub update is a pull request to the app's Flathub
  repository. Weekly, on the day the Store package is submitted (Monday),
  from the same commit; a manual run for urgent fixes.

**glibc baseline.** An executable built on Ubuntu 24.04 needs glibc 2.39
(R1) and would not start on Ubuntu 22.04, Debian 12 or Mint 21. The
AppImage is built on `ubuntu-22.04` runners (glibc 2.35), which covers
Ubuntu 22.04+, Debian 12+, Fedora 36+ and Mint 21+ (versions stated from
general knowledge; openSUSE Leap's glibc is not known here; both checked in
T141). CI fails when the highest `GLIBC_` symbol exceeds 2.35.
**Assumption A-19**: GitHub still offers `ubuntu-22.04` and
`ubuntu-22.04-arm` runners when Lot 14 lands; otherwise the build runs in a
`debian:12` container. Task T141. Flatpak brings its own glibc.

**Architectures.** x86_64 and aarch64 for both channels: Flathub builds
both by default, and GitHub hosts arm64 Linux runners for public
repositories. aarch64 has no Tier 1 desktop to test on unless the owner has
one (**open question Q8**); it ships as Tier 2.

**Signing and attestations.** The repository signs nothing on Windows yet
(`docs/code-signing.md`) and attests every release file with
`actions/attest` (SLSA provenance, `release.yml`). Linux does the same: the
AppImage, `releases.linux.json` and the `.nupkg` files are attested in the
same step and listed in `Bientot.intoto.jsonl`. No GPG signature of the
AppImage at launch: nothing verifies one at run time, and Velopack checks
the package hashes of the feed. Flathub signs its own repository; the
listing is verified through the domain.

**Size budget.** The gate stays 20 MB, the same number as Windows, on the
stripped ELF executable of the `release` and `direct` builds for x86_64
(`stat -c %s`), failing the `Rust on Linux` job; the aarch64 executable and
the AppImage file are reported and gated at 20 MB in the release workflow.
Measured today without any Linux backend: 15.54 MB (R1). Expected
additions: `ksni` and the D-Bus proxies (`zbus` is already linked through
GPUI), `rustls` in place of dynamic OpenSSL, Velopack in `direct`.
Startup (ready file under 1 s) and memory (under 60 MB) are reported, not
enforced, as on Windows: hosted runners have no GPU.

## R14. Plugin runtime

**Code.** `wasmi` 2.0 is pure Rust; `bientot-plugins`, `-api`, `-sdk` and
`-tool` contain no `cfg(windows)` at all, and their tests, the hostile
corpus's Linux memory measure and the fuzz targets already run on Ubuntu.
What is OS-bound in the plugin feature sits in `bientot-app`:

| Item | Windows | Linux |
| --- | --- | --- |
| Folder | `%LOCALAPPDATA%\Bientot\plugins\` | `<data folder>/plugins/` |
| « Open folder » real path | `GetFinalPathNameByHandleW` for MSIX | Not needed |
| Install from file, drop target | Common dialog; GPUI drop | File chooser portal; GPUI drop (to verify on Wayland, T128) |
| Revocation list fetch | `ureq` with the update check | Same; in Flatpak the daily check does not exist, so the fetch gets its own daily timer (T128) |
| `--no-plugins`, safe mode | Same code | Same code |
| Hostile corpus in CI | `Rust on Windows`, release build | Also in `Rust on Linux` |
| Store policy | Microsoft Store 10.2.2 (spec 003, R8) | Flathub has no equivalent rule on sandboxed WebAssembly; the listing still describes plugins |

The docs spell plugin paths with backslashes; they become
platform-neutral (T182).

## R15. CI and release

- **Pull requests.** `Rust` (Ubuntu) keeps linting and testing everything
  and gains the Windows cross-lint that `CONTRIBUTING.md` describes for
  local use (clang, `llvm-lib`, stand-in C compiler), so that a pull
  request that skips `Rust on Windows` still type-checks every
  `cfg(windows)` line. A new **`Rust on Linux`** job mirrors `Rust on
  Windows`: release build, size gate, glibc gate, the hostile corpus, a
  start under a headless session, and the desktop integration tests.
- **Headless desktops.** X11: `Xvfb` with `openbox` (a work area and
  focus). Wayland: `sway` with `WLR_BACKENDS=headless`, which offers
  layer-shell and ext-idle-notify (wlroots); Weston offers neither
  (**Source**). D-Bus: `dbus-run-session` with `python-dbusmock` standing
  in for logind, the notification server and Mutter's IdleMonitor, and a
  small SNI host written with `zbus` for the tray. Rendering: Mesa's
  software Vulkan, as the screenshots job does. GNOME Shell and KDE Plasma
  do not run on a hosted runner in any maintainable way: those are
  validated by hand (quickstart) on a virtual machine or the owner's
  machine, before each release that touches the Linux shell.
- **Screenshots.** `bientot-screenshots` already renders on Linux; the
  scenes are platform-neutral except the strings that name Windows. A
  `--platform linux` switch renders the Linux variants of those scenes
  (T124), so the panel reviews both.
- **Release.** One release, one version, both platforms. `release.yml`
  gains `build-linux` jobs (x86_64, aarch64) beside the Windows `build`;
  `publish` waits for all of them and attests every file. The draft-then-
  publish step already keeps a half-uploaded release invisible. A failed
  Linux build blocks the night's release for Windows too:
  **open question Q6**.
- **Release notes.** A summary line may concern one platform. A file of
  `scripts/release-summary/` gains an optional first line `platforms:
  windows` or `platforms: linux`; `check-release-summary.sh` accepts it and
  `release-notes.sh` prints such lines under « Windows » or « Linux » after
  the common ones. Until the first Linux release, Linux-only pull requests
  are titled `build(app)`, as the plugin runtime's were while it was
  compiled out (#160–#187), and the launch is one `feat(app)` with its
  summary and an announcement (T186). **Open question Q7**.

## R16. Site, usage signal, listings

- **Download.** `/download` answers by `User-Agent`: Linux gets the page
  section with Flathub and the AppImage (x86_64 or aarch64 by
  `navigator.userAgentData` or the UA string), Windows gets `Setup.exe` as
  today; `/download/windows`, `/download/linux` and
  `/download/linux-arm64` are explicit. The page shows the other platform
  as a secondary link, never hides it. `releases.ts` learns
  `releases.linux.json`, `Bientot-x86_64.AppImage`,
  `Bientot-aarch64.AppImage` and the Linux `.nupkg` names.
- **Usage signal.** The service accepts exactly three keys
  (`ping.ts`). Linux needs `channel` values `flatpak` and `appimage`, which
  already tell the platform. Knowing the desktop (GNOME, KDE, other) and
  the session type (Wayland, X11) would tell where to spend testing effort,
  but it is new data leaving the machine: Principle I asks for a product
  decision, and `docs/privacy.md` and SC-005 of spec 001 name the fields.
  **Open question Q3**; the recommended shape is in
  `contracts/usage-signal.md`. The statistics page and the PostHog saved
  query (`usage_daily`, version 1) gain the same dimensions under the
  existing small-number suppression.
- **Listings.** AppStream metainfo
  (`packaging/linux/dev.bientot.Bientot.metainfo.xml`) with summary and
  description in English and French, screenshots (**Source**: Flathub
  requires them, hosted at stable URLs), release entries generated from the
  release summaries, content rating, `<requires>` a desktop display. The
  text reuses `packaging/store/listing.json`. Desktop entry with
  `StartupWMClass` and the app id; icons 64, 128, 256 and scalable.

## Assumptions and the tasks that settle them

| Id | Assumption | Task |
| --- | --- | --- |
| A-1 | X11 override-redirect flyout gets focus and reports its loss | T041 |
| A-2 | `_NET_WORKAREA` and RandR give the work area of the clicked monitor on Cinnamon, Xfce and GNOME on Xorg | T042 |
| A-3 | Layer-shell `OnDemand` focus and focus loss on KWin, Sway, Hyprland | T043 |
| A-4 | Mutter places the undecorated flyout acceptably | T044 |
| A-5 | SNI `Activate` coordinates are usable on X11 hosts and absent on Wayland hosts | T041, T043 |
| A-6 | `ksni` works in Flatpak with the watcher talk name only | T131 |
| A-7 | Tier 1 KWin offers ext-idle-notify version 2 | T072 |
| A-8 | Flathub accepts `org.gnome.Mutter.IdleMonitor` | T136 |
| A-9 | `LockedHint` is kept by GNOME, KDE and Cinnamon | T082 |
| A-10 | Flathub accepts `--system-talk-name=org.freedesktop.login1` | T136 |
| A-11 | The default user can read the system journal on Ubuntu, Fedora and Mint | T091 |
| A-12 | Velopack's AppImage update flow works on Tier 1, x86_64 and aarch64 | T140 |
| A-13 | Behaviour without a Vulkan driver | T031 |
| A-14 | Windows zone names need a mapping on Linux | T125 |
| A-15 | Which Settings portal key carries reduced motion | T123 |
| A-16 | Orca reads the flyout through AT-SPI | T127 |
| A-17 | Notification body clicks arrive, directly and through the portal | T101 |
| A-18 | The Background portal does not prompt again once granted | T112 |
| A-19 | `ubuntu-22.04` runners still exist | T141 |
| A-20 | `rustls` costs under 1.5 MB | T037 |

## Sources

Read on 24 November 2026.

- Velopack, « Linux Overview »:
  <https://docs.velopack.io/packaging/operating-systems/linux>
- `velopack` 1.2.161 crate source (`locator.rs`, `constants.rs`,
  `manager.rs`), `tray-icon` 0.26.1 (`Cargo.toml`,
  `platform_impl/ksni/mod.rs`), `ksni` 0.3.5 (`lib.rs`), downloaded from
  crates.io.
- `gpui-pre` and `gpui-pre-linux` 0.3.9 in the Cargo registry.
- `tray-icon` on crates.io (features and Linux dependencies):
  <https://crates.io/crates/tray-icon>; tauri-apps/tauri issue 11293.
- Wayland Explorer, ext-idle-notify-v1 compositor support:
  <https://wayland.app/protocols/ext-idle-notify-v1>
- systemd, `org.freedesktop.login1`:
  <https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html>
- Flatpak, « Desktop Integration » (StatusNotifier):
  <https://docs.flatpak.org/en/latest/desktop-integration.html>
- Flathub, linter and requirements:
  <https://docs.flathub.org/docs/for-app-authors/linter>,
  <https://docs.flathub.org/docs/for-app-authors/requirements>,
  <https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines>
- util-linux issue 3598 (future of `last` and `wtmp`):
  <https://github.com/util-linux/util-linux/issues/3598>
- GNOME Discourse, « IdleMonitor (DBus) and unprivileged user »:
  <https://discourse.gnome.org/t/idlemonitor-dbus-and-unprivileged-user/1120>

Not verified by any source here and stated from general knowledge: which
distributions ship the AppIndicator extension, the group that may read the
journal on each distribution, Cinnamon's `org.cinnamon.Muffin.IdleMonitor`
name, the `SD_MESSAGE_SLEEP_*` journal entries as the suspend record, GitHub's
arm64 runner offer, and the glibc version of each distribution. Each is
checked by the task of its lot before the code relies on it (T042, T072,
T091, T141).
