---

description: "Task list for Linux support"
---

# Tasks: Linux support

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`

**Tests**: required. Lots 7 to 9 produce accounting evidence (constitution,
Principle II); Lots 1 and 2 move accounting code and must prove they change
nothing.

Each task names its files. **[P]** tasks touch different files and can run in
parallel. Each pull request follows `CONTRIBUTING.md`: Conventional Commits
with a scope (`app`, `site`, `repo`), French and English texts, and a file
under `scripts/release-summary/` with one `fr:` and one `en:` line for every
`feat(app)`, `fix(app)` and `perf(app)` pull request. Until the launch
(T186), Linux pull requests are `build(app)`, `refactor(app)`, `test(app)`,
`ci(repo)` or `docs(repo)` and owe no release line (`plan.md`,
« Conventions »). Suggested pull requests are marked **PR**; a lot of
`lots.md` is one phase here.

Three marks beside the story:

- **[UI]**: changes something the user sees or reads. Goes through the
  three-reviewer panel (designer critique, UX writing, screenshot review)
  and updates the GPUI mockups and the design system in the same pull
  request. Texts start from `contracts/copy.md`.
- **[DESKTOP]**: cannot be proven by CI; needs a real Linux desktop (a
  virtual machine with a full desktop session counts, unless the task says
  hardware). The task says where and what to observe; the result goes to
  `research.md`, « Validation results ».
- **[OWNER]**: needs an account, a secret or a decision only the owner has.

Every lot keeps Windows shippable: its pull requests pass `Rust` and `Rust
on Windows`, and the Windows executable changes only through code both
platforms share.

## Phase 0: Ratification (Lot 0)

**PR 0** `docs(repo): ratify platform parity in the constitution and D-026`.
Blocks Lot 2 (the gate cites the principle) and Lot 9.

- [ ] T001 [OWNER] Record the owner's answers to Q1–Q14 of `specs/005-linux-support/lots.md` in a « Clarifications » section of `spec.md`; adjust the parity matrix, `contracts/` and the tasks of the lots each answer changes.
- [ ] T002 Amend `.specify/memory/constitution.md` to 1.3.0 with the text of `plan.md` (Principles III and V, the platform constraint, new Principle VII); update the sync impact report at its head.
- [ ] T003 Add D-026 to `docs/product-decisions.md` with the text of `plan.md`, corrected by the answer to Q4.
- [ ] T004 [P] Create `docs/platforms.md` from `spec.md` (supported desktops with tiers, parity matrix); link it from `README.md` under a « Linux: in progress » line that promises no date.

**Checkpoint**: the constitution names two platforms; D-026 exists.

## Phase 1: Portable platform logic leaves `bientot-windows` (Lot 1) — US1

**PR 1** `refactor(app): move the portable platform logic into bientot-platform`.
No behaviour change. May be split in two pull requests at T012.

- [x] T010 [US1] Create `crates/bientot-platform/` (no OS dependency). Move with their tests, unchanged: `activity.rs` (`LastInputProbe`, `reading_from_ticks`, `BuiltInActivitySource`), the recorder half of `lifecycle.rs` (`LifecycleChange`, `LifecycleTransition`, `LifecycleEventRecorder`, `LifecycleHooks`), `PowerState`, `dotnet_ticks`, the service half of `updates.rs`, the request half of `notifications.rs`, the protocol half of `single_instance.rs`, `StartupRegistrationState` and `LaunchIntent` from `startup.rs`. `bientot-windows` depends on it and keeps `native` modules, `decode` and the Win32 constants.
- [x] T011 [US1] Rename in shared code what names Windows and is not stored data: `LaunchIntent::WindowsStartup` → `SignIn`, `UpdateStore` → `UpdateSource`; keep the flag `--worktime-windows-startup` and every ledger origin string. Add `reading_from_idle` with tests against `reading_from_ticks` (wraparound, idle longer than uptime).
- [x] T012 [US1] Introduce `PowerBoundaryLog` and `SessionBoundaryLog` (`contracts/platform-ports.md`) in `bientot-platform`; move pairing, idempotent recording and the « Avant Bientôt » recovery above them; implement both in `bientot-windows` by wrapping the existing readers and record parsers (`SystemEventLog`, `WinlogonEventLog` become private to it). The existing fixture tests must give byte-identical ledger rows.
- [x] T013 [US1] Move `StartupPort` from `crates/bientot-app/src/settings/startup.rs` and `MenuEntry`, `item_id`, `chosen` from `crates/bientot-app/src/tray_model.rs` to `bientot-platform`; add `StartupPort::system_page`. Update `docs/architecture.md` (crate map).

**Verification**: `cargo test --workspace` on both jobs with no test body edited (`git diff --stat` on test files shows `use` lines only); Windows executable size within 50 KB of `main`; `Rust on Windows` green, including package and toast scripts.

**Checkpoint**: `bientot-windows` contains only code that needs Windows, plus its decoders.

## Phase 2: One composition root in the app (Lot 2) — US1, US8

**PR 2** `refactor(app): build the app on platform ports from one composition root`.
Depends on Lot 1. No behaviour change.

- [x] T020 [US1] Add `crates/bientot-app/src/platform/mod.rs`: `PlatformContext`, `Platform`, `Platform::null()`, `build` (`contracts/platform-ports.md`); `null.rs` for other systems. Add the remaining ports to `bientot-platform`: `LifecycleSource`, `InstanceLock`, `Tray`, `Desktop`, `ReducedMotion`, `SystemInfo`, `NotificationClicks`, `wait_for_exit`, each with an in-memory implementation.
- [x] T021 [US1] Move `mod native` of `crates/bientot-app/src/main.rs` and its 40 conditional lines into `platform/windows.rs`; `main.rs` calls `platform::build` once. Delete `NoLastInput`.
- [x] T022 [US1] Move `crates/bientot-app/src/tray.rs` into `platform/windows.rs` (or `platform/windows/tray.rs`) behind `Tray` and `Desktop`; `host.rs` publishes through `Tray::publish`; `shell.rs` asks `Desktop::flyout_placement` and maps `FlyoutPlacement` to window options without a condition.
- [x] T023 [P] [US1] Remove the conditions of `notify.rs`, `updates.rs`, `file_dialogs.rs`, `zones.rs`, `diagnostics.rs`, `transfer.rs`, `plugins.rs`, `launch.rs`, `lib.rs` and `settings_view.rs` by taking the matching port; « Open Startup apps » shows when `system_page()` is `Some`.
- [ ] T024 [US8] Add `scripts/check-platform-cfg.sh` and `scripts/platform-cfg-allowlist.txt` (`plan.md`, « No scattered conditions »); run it in the `Rust` job of `.github/workflows/ci.yml`; add a self-test that a seeded violation fails.
- [ ] T025 [US8] Document the rule in `CONTRIBUTING.md` (« Platform code ») with the local command.
- [ ] T026 [US1] [DESKTOP] Windows pass of `quickstart.md` on a real Windows machine: tray on light and dark taskbar, flyout on two monitors at different scales, toast click with the app closed, autostart, lock and sleep in the day, update prompt. Screenshots job output identical to `main`.

**Checkpoint**: US1 scenarios 1–3 pass. `grep` finds no platform condition in `bientot-app` outside `src/platform/`.

## Phase 3: Linux in every pull request (Lot 3) — US8

**PR 3a** `ci(repo): build, measure and start Bientôt on Linux in every pull request`.
Depends on Lot 2 (for the gate; the job itself could land earlier).

- [x] T030 [US8] Add the `Rust on Linux` job to `.github/workflows/ci.yml`, with the path filter of `Rust on Windows`: release build of `release` then `direct` features, stripped size reported against 20 000 000 bytes (`stat -c %s`) and a warning above it until the Linux size task (issue #226) makes it a gate, summary table as the Windows job prints. Fix the system package list in `CONTRIBUTING.md`. Done: the job runs on the files of `Rust` plus its own two scripts, so every Rust change is built and started on Linux; it builds `release` only, since `direct` compiles the same code on Linux until T143 and T145 add its build; the size is reported, not enforced, until the Linux size task (issue #226). `gpui_platform` gains its `x11` and `wayland` features on Linux, without which GPUI opens no window (`research.md`, R1).
- [x] T031 [US8] Headless start in that job: `Xvfb` with `openbox` and Mesa software Vulkan, `--data-dir` on a temporary folder, ready file within 10 s, nothing on standard error; capture a screenshot of the flyout (`--open-flyout`) as an artifact. Run once without any Vulkan driver and record what happens (assumption A-13); file the follow-up for `NoGraphics.Text` (T117) from the result. Done: GPUI draws through OpenGL with no Vulkan driver, and nothing, silently, with no graphics API at all (`research.md`, R1, A-13; T117).
- [x] T032 [US8] Add the Windows cross-lint to the `Rust` job: `cargo clippy --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings` with the tools `CONTRIBUTING.md` lists; cache them. Done in `scripts/lint-windows.sh`: the workspace, then `bientot-app` with each feature set the Windows job builds, never `--all-features`, whose `screenshots` feature builds only on Linux.
- [x] T033 [P] [US8] Run the hostile plugin corpus on the Linux release build in `Rust on Linux`.
- [x] T034 [US8] Add `scripts/linux-desktop-harness.sh {x11|wayland}`: starts `dbus-run-session`, `python-dbusmock` templates for logind, notifications and Mutter's idle monitor, a test StatusNotifier host (small `zbus` program under `crates/bientot-linux/tests/support/`), and either `Xvfb` + `openbox` or `sway` with `WLR_BACKENDS=headless`; runs the tests marked `#[ignore = "desktop-harness"]`. Empty test list for now; the job calls it. Done without the test StatusNotifier host, deferred to Lot 4, and the mock of Mutter's idle monitor, deferred to Lot 7.
- [x] T035 [P] [US8] Add `scripts/measure-performance.sh` (ready-file time, resident memory at rest) and report both in the job summary, not enforced.
- [ ] T036 [US8] [OWNER] Update `docs/ci-quality-gates.md`; add `Rust on Linux` to the required checks of the branch protection.

**PR 3b** `build(app): use rustls for HTTPS on Linux`.

- [x] T037 In `crates/bientot-app/Cargo.toml` (and any crate that enables `ureq`'s TLS), use `rustls` with the platform verifier under the Linux target and keep `native-tls` under Windows; add the « no `libssl` » gate to `Rust on Linux`; state the size before and after (assumption A-20: under 1.5 MB). If it costs more, keep OpenSSL for the Flatpak, bundle nothing, and raise it as a question before Lot 14. Done 2026-11-25: `bientot-app` is the only crate with `ureq`; it takes `rustls` with `ring` and `platform-verifier` on every target but Windows, so macOS builds keep a TLS, and `platform::system_tls` picks the provider for the usage signal, the revocation list and the Velopack feed. The stripped `release` executable grows from 24 079 368 to 24 993 032 bytes (+0.91 MB), `direct` by the same; `ldd` lists neither `libssl` nor `libcrypto`, the job fails if it does, and `libssl-dev` leaves the package list. A-20 holds (`research.md`, R12). A probe with the same `ureq` configuration, through the container's HTTPS proxy, completes a handshake with `worktime.thanaen.dev` and refuses `expired.badssl.com`. Not verified: the app's own requests end to end; and the Linux notices, since `bientot-notices` reads the Windows tree only, although the licenses of the new crates (`ring`'s Apache-2.0 AND ISC, `webpki-roots`'s CDLA-Permissive-2.0) are all in `license::ALLOWED`.

**Checkpoint**: US8 scenarios 1–3 pass. A Linux executable is built, measured and started on every pull request that touches Rust.

## Phase 4: Tray and flyout spike on real desktops (Lot 4) — US2, US3

**PR 4** `build(app): add the Linux crate and a tray and flyout spike`.
Depends on Lot 3. The spike is `crates/bientot-linux/examples/shell-spike.rs`: a `ksni` item whose click opens a GPUI window of the flyout's size by each placement of `contracts/linux-backends.md`, logging focus and bounds. It is not shipped. This lot exists to turn assumptions A-1 to A-5 into facts before Lots 5 and 6 are built on them.

- [x] T040 [US2] Create `crates/bientot-linux/` (portable half on any OS, `native` under `cfg(target_os = "linux")`), add `ksni`, `x11rb` features `randr` and `screensaver`; write the spike example. State the size effect. Done 2026-11-26: `placement.rs` holds the monitor under the click, the work area (`_NET_WORKAREA` clipped to it) and the nearest corner, tested on any OS; its `native` half reads RandR and `_NET_WORKAREA` with `x11rb`. `ksni` 0.3.7 (Unlicense, in `license::ALLOWED`) runs on the async-io `zbus` GPUI already builds, and adds four packages to the lock. `bientot-app` does not depend on the crate yet, so its stripped `release` executable is unchanged: 24 993 032 bytes, the same SHA-256 before and after. The spike, `examples/shell-spike.rs`, takes a placement (`popup`, `popup-focus`, `normal`, `layer`, `layer-exclusive`, `compositor`) and is never shipped. The harness gains a StatusNotifierWatcher mock (`tests/support/status_notifier_watcher.py`, a python-dbusmock template rather than the `zbus` program T034 planned: the harness already runs python-dbusmock, and a test only needs the watcher's D-Bus side). Headless results are in `research.md`, « Spike results ».
- [ ] T041 [US2] [DESKTOP] X11, on Linux Mint Cinnamon and one of Xfce or GNOME on Xorg: does the override-redirect `PopUp` take keyboard focus after `activate_window`, report deactivation on a click outside, and receive Esc (A-1)? Does `Activate(x, y)` give screen coordinates (A-5)? If A-1 fails, test the fallback (undecorated `Normal` window with skip-taskbar and above) in the same session. Headless, 2026-11-26 (openbox, Mutter without GNOME Shell): GPUI's `PopUp` alone never gets the focus; with `SetInputFocus` from the app (`shell-spike popup-focus`) it does, and Esc and a click in another window close it; the fallback works under openbox but flaps (focus in, then out within 3 ms) on 3 opens of 5 under Mutter. xfce4-panel's `Activate` gives root coordinates. On the desktops, run `popup`, `popup-focus` and `normal`.
- [ ] T042 [US2] [DESKTOP] X11, same desktops plus a two-monitor setup: do `_NET_WORKAREA` and RandR give the work area of the clicked monitor, with the panel at the top and at the bottom (A-2)? Headless, 2026-11-26 (xfce4-panel on openbox): right with the panel at the top and at the bottom on one monitor; with two RandR monitors and a panel on one, `_NET_WORKAREA` takes the panel's height off both, so the other monitor's work area comes out 33 px short. Check whether Muffin, xfwm4 and Mutter publish `_GTK_WORKAREAS_D<n>`.
- [ ] T043 [US2] [DESKTOP] Wayland, on KDE Plasma 6 and Sway (Hyprland if at hand): does a layer-shell surface with `OnDemand` keyboard get focus, lose it on a click outside, and close on Esc (A-3)? What does the host send as `Activate` coordinates (A-5)? Which corner is right by default? Headless, 2026-11-26 (sway 1.9): an `OnDemand` surface gets no focus at map, only when the pointer enters it or a click lands in it, and stays open after a click elsewhere if it never had the focus; `Exclusive` (`shell-spike layer-exclusive`) has it at map. Esc was not testable. waybar sends the click's position on the output, not 0, 0, which the contract's Wayland rule drops.
- [ ] T044 [US3] [DESKTOP] [UI] Wayland, on Ubuntu 24.04 GNOME and Fedora GNOME: where does Mutter place the undecorated flyout, does it close on focus loss, is the result acceptable (A-4)? Screenshots go to the designer reviewer; the outcome may change the GNOME row of the parity matrix. Headless, 2026-11-26: not measurable (headless Mutter has no input, and a client cannot read its position); Mutter refuses layer-shell, so `open_window` fails and the app must fall back to the compositor-placed window. GPUI cannot hand the tray host's activation token to the window, so whether GNOME focuses it is the first thing to look at (`shell-spike compositor`).
- [ ] T045 [US2] Write « Spike results » in `research.md`; correct `contracts/linux-backends.md`, the parity matrix and the tasks of Lots 5 and 6 in the same pull request. The headless part of « Spike results » is written (2026-11-26); the contract and the later lots wait for T041 to T044. Candidates so far: on X11, `bientot-linux` sets the input focus on the `PopUp` itself; on Wayland, `at` keeps a non-zero position from the host; the app ignores a deactivation while the X input focus is still on the flyout.
- [ ] T046 [US2] [OWNER] If a Tier 1 desktop fails with no fallback, stop and bring the options to the owner before Lot 5. Nothing headless calls for it yet: every X11 failure seen has a remedy that worked under both window managers.

**Checkpoint**: A-1 to A-5 are facts. The riskiest part of the feature is known.

## Phase 5: The tray icon (Lot 5) — US2

**PR 5** `build(app): show the tray icon and its menu on Linux`. Depends on Lot 4.

- [ ] T050 [US2] In `crates/bientot-linux/src/tray.rs`: `MenuEntry` → dbusmenu tree (portable, tested: labels, separators, checked and disabled states, submenus, ids from `item_id`), RGBA → ARGB32 pixmaps at 16, 22, 24, 32, 48 px.
- [ ] T051 [US2] `native`: the `ksni` item implementing `Tray`; `Activate` → `TrayEvent::Activated`, menu clicks → `MenuChosen`; `watcher_online` / `watcher_offline` / `WontShow` → `HostChanged`, with the 5 s grace of `data-model.md`.
- [ ] T052 [US2] `crates/bientot-app/src/platform/linux.rs`: build the tray, forward events to the shell as `platform/windows.rs` does.
- [ ] T053 [US2] [UI] Icon for a panel of unknown colour: design the outline in the mockups at 16, 22, 24 and 32 px on light and dark panels (GNOME top bar, KDE panel light and dark, Cinnamon), implement it in `crates/bientot-app/src/tray_art.rs` behind `Tray::panel_is_dark() == None`, add the scenes to `bientot-screenshots`, update the design system (« Tray icon »).
- [ ] T054 [P] [US2] Harness tests against the test host: item registers, pixmaps and tooltip arrive, a menu click reaches the app, the host leaving and returning flips `TrayHost` after the grace only.
- [ ] T055 [US2] [DESKTOP] On the four Tier 1 desktops: icon legible, tooltip, primary click, secondary-click menu identical to the Windows reference, behaviour when the shell restarts (`plasmashell --replace`, GNOME extension off and on).
- [ ] T056 [US2] Diagnostics: `tray.backend`, `tray.host` in the log and the export.

**Checkpoint**: on a desktop with a host, the icon and menu work; the flyout still opens as a plain window.

## Phase 6: Flyout placement and window mode (Lot 6) — US2, US3

**PR 6a** `build(app): place the flyout on X11 and Wayland`. **PR 6b** `build(app): run in a window when the desktop has no tray`. Depend on Lot 5.

- [ ] T060 [US2] `crates/bientot-linux/src/placement.rs`: pure corner and bounds maths (work area, monitor, click, margin), tested with the layouts recorded in T042; `placement/x11.rs` reads `_NET_WORKAREA` and RandR.
- [ ] T061 [US2] `placement/wayland.rs`: layer-shell detection and the `Layer` placement; default corner per desktop.
- [ ] T062 [US3] `CompositorPlaced` for GNOME on Wayland, with whatever T044 established.
- [ ] T063 [US3] Window mode (FR-012): `FlyoutPlacement::Window`, minimize instead of close on focus loss, re-open on a second launch, live switch on `HostChanged`; `windowModeNoticeDismissed` in `app-state.json`.
- [ ] T064 [US2] [UI] The flyout draws its own radius and shadow on a transparent window when `Desktop::system_draws_frame()` is false (square without a compositing manager); the window-mode notice (`WindowModeNotice.*` of `contracts/copy.md`) and the look of window mode; the GNOME centred flyout. Mockups, design system (« Flyout », new « Window mode » pattern), screenshot scenes.
- [ ] T065 [P] [US2] Harness tests: under `openbox` the window's bounds equal the computed ones for panel top and bottom; under `sway` a layer surface exists with the right anchor; without a host the window is a normal toplevel.
- [ ] T066 [US2] [DESKTOP] Steps 4 to 10 of `quickstart.md` on the four Tier 1 desktops, including scales 100 %, 125 %, 200 % and two monitors.

**Checkpoint**: US2 scenarios 1–3 and 6 pass; US3 scenarios 1–2 pass.

## Phase 7: Activity (Lot 7) — US2, US3

**PR 7** `build(app): detect activity on Linux`. Depends on Lot 3; independent of Lots 4–6.

- [ ] T070 [US2] `crates/bientot-linux/src/idle.rs`: the selection order of `contracts/linux-backends.md` as a pure function of probes (tested for every session), and `IdleProbe` implementing `LastInputProbe` through `reading_from_idle`.
- [ ] T071 [US2] `idle/ext_idle.rs`: second Wayland connection, `ext_idle_notifier_v1`, version 2 input notification with the version 1 fallback and its log line; idle time between events computed from the last event.
- [ ] T072 [US2] [DESKTOP] On KDE Plasma 6 (Tier 1) and Sway: the advertised version of the protocol (A-7); step 13 of `quickstart.md` (a full-screen video does not look like activity). Record versions in `research.md`.
- [ ] T073 [P] [US2] `idle/mutter.rs`: Mutter and Muffin idle monitors over `zbus`.
- [ ] T074 [P] [US2] `idle/screensaver.rs` (with the unit check) and `idle/xss.rs` (`MIT-SCREEN-SAVER`).
- [ ] T075 [US3] [UI] No backend: the built-in source reports itself unhealthy as on a failed Windows query; the flyout shows `ActivityUndetectable.*`; mockups, design system, screenshot scene. `idle.backend` in the log, the export and Settings › Diagnostic.
- [ ] T076 [US2] Tests: harness (dbusmock idle monitor; `sway` with synthetic input; `Xvfb` with `xdotool`); **golden ledger**: one recorded timeline of idle readings fed to `BuiltInActivitySource` through `reading_from_ticks` and through `reading_from_idle` writes identical rows (SC-001).
- [ ] T077 [US2] [DESKTOP] Steps 11 and 12 of `quickstart.md` on the four Tier 1 desktops.

**Checkpoint**: US2 scenario 4 and US3 scenario 3 pass.

## Phase 8: Lock, sleep, session (Lot 8) — US2, US3

**PR 8** `build(app): record lock, unlock, sleep and resume on Linux`. Depends on Lot 3.

- [x] T080 [US2] `crates/bientot-linux/src/logind.rs`: own-session resolution; pure mapping of property changes and signals to `LifecycleChange` (tested: `LockedHint`, `Active`, both, `PrepareForSleep`, duplicates). Done 2026-11-26: `SessionTracker` is a reducer over the session's `Active`, a lock signal (`LockedHint`, the screensaver, or none) and the power; `apply(Signal) -> Option<LifecycleChange>` gives a change only when a signal changes what is known and the state differs from the one last reported. `SessionReadings` seeds it and `snapshot()` follows `data-model.md` (`Unavailable` implies `Unknown`). Tested on any OS, the fallback included. The resolution (process id, `XDG_SESSION_ID`, the user's `Display` session) needs the bus, so it lives in `logind/native.rs` and the harness tests it.
- [x] T081 [US2] `native`: the system-bus thread implementing `LifecycleSource`; snapshot at start; the system bus closing and SIGTERM end the observation cleanly. Done 2026-11-26: `LogindSource<C, Z>` runs `zbus` 5.19 (`async-io`, the version and features GPUI and `ksni` already build; `async-io`, `futures-lite` and `async-channel` are in the lock too, so no package is added) on a thread of its own: match rules first, then the readings, then one ordered stream per bus. The system bus closing calls `ended()`; `PrepareForShutdown` is not watched, since logind sends `PrepareForShutdown(false)` after `true` when the shutdown fails and the session goes on. Dropping the watch stops and joins the thread. SIGTERM keeps its default action: every change is in the ledger as it arrives, and a handler in the source would delay the logout; making the app quit on it, so that the activity observation ends too, belongs to the app and no task does it yet (`research.md`, R7). `bientot-app` does not use the source yet (T052 creates `platform/linux.rs`), so its `release` executable is unchanged.
- [ ] T082 [US2] [DESKTOP] On GNOME, KDE Plasma and Cinnamon (Tier 1), and on Sway with `swaylock`: does `LockedHint` follow the lock screen (A-9)? Does `Active` change on Switch User? Fix the list of desktops with live lock reporting in `contracts/linux-backends.md`. Headless, 2026-11-26: python-dbusmock's logind only, so nothing about real desktops; the list is empty, and a desktop on it would start with `LockedHint` as its lock signal. Also check whether GNOME's `ActiveChanged(true)` fires on a blank screen with the automatic lock off, which the fallback would record as a lock.
- [x] T083 [P] [US3] Fallback on `ActiveChanged` of the screensaver interfaces; `LockReporting::Unavailable` otherwise. Done 2026-11-26: `org.freedesktop.ScreenSaver` and `org.gnome.ScreenSaver` on the session bus, `GetActive` read at start only when the name has an owner; the first `LockedHint` seen true takes over for good. `LockReporting` already existed in `bientot-platform` (Lot 2). The port cannot report `LockReporting` changing after the snapshot, which T084 will need (`research.md`, R7).
- [ ] T084 [US3] [UI] `LockStateUnknown.Text` in Settings › Diagnostic and the export; mockup, screenshot scene.
- [x] T085 [US2] Tests: dbusmock logind in the harness; **golden ledger**: the same sequence (lock, unlock, suspend, resume, disconnect) through the Windows decoder and through the logind mapping writes identical rows (SC-001). Done 2026-11-26: four `desktop_harness` tests in `logind/native/tests.rs` (resolution order, lock, Switch User, sleep, a lock after a failed shutdown, another session ignored, the screensaver fallback, dropping the watch), on X11 and Wayland. The mock gets a `GetSessionByPID` and a working `GetUser` from the tests. `tests/golden_ledger.rs` runs the real Windows decoder (`bientot_windows::lifecycle`, whose decoding half builds on any OS; a dev-dependency only) and `SessionTracker` over one GNOME-like timeline (lock, unlock, suspend, resume, Switch User and back) into two ledgers and compares every column of every row, on every CI system.
- [ ] T086 [US2] [DESKTOP] Steps 14 to 17 of `quickstart.md` on the four Tier 1 desktops; suspend needs hardware or a virtual machine that supports S3. Headless, 2026-11-26: `PrepareForSleep` sent by the mock reaches the listener once each way, and a lock after `PrepareForShutdown(true)` then `(false)` is still recorded; whether a real resume sends `PrepareForSleep(false)` before or after the desktop clears `LockedHint` decides the order of the resume and unlock rows.

**Checkpoint**: US2 scenario 5 and US3 scenario 4 pass. With Lots 5–7, a day is counted on Linux.

## Phase 9: History before the start (Lot 9) — US7

**PR 9** `build(app): recover power and session history on Linux`. Depends on Lot 8, on D-026 (T003) and on Q4.

- [ ] T090 [US7] `crates/bientot-linux/src/journal.rs` fallback: boot time from `/proc/stat`, sleep total from the two clocks, as a `PowerBoundaryLog` with origin `linux-boot-time`.
- [ ] T091 [US7] [DESKTOP] On Ubuntu 24.04, Fedora and Linux Mint, as the default user: can the system journal be read (A-11)? Which `MESSAGE_ID` and fields mark suspend, resume, shutdown and boot? Save real `journalctl --output=json` extracts (scrubbed of host names) as fixtures under `crates/bientot-linux/tests/fixtures/`. Fix the ids in `contracts/linux-backends.md`.
- [ ] T092 [US7] Journal reader: parse the fixtures to `PowerBoundary` (portable, tested); `native` runs `journalctl`; `AccessDenied` and `NoSource` paths; origin `linux-power-history`.
- [ ] T093 [US7] `SessionBoundaryLog` from logind's `Timestamp`, `LockHistory::NotRecorded`; the recovery follows D-026 (tests: with sleep, without journal, unknown opening, Flatpak).
- [ ] T094 [US7] [UI] « Avant Bientôt » on Linux: the `FlyoutRebuiltReviewText` variant and `PowerHistoryPartial.Text`; mockups, design system, screenshot scenes; `docs/session-history-recovery.md` and `docs/power-history-recovery.md` gain a Linux section.
- [ ] T095 [US7] [DESKTOP] Steps 18 and 19 of `quickstart.md` on Ubuntu and Fedora.

**Checkpoint**: US7 scenarios pass.

## Phase 10: Notifications (Lot 10) — US4

**PR 10** `build(app): show reminders as desktop notifications on Linux`. Depends on Lot 3.

- [ ] T100 [US4] `crates/bientot-linux/src/notifications.rs`: `NotificationSink` over `notify-rust` with the hints of `contracts/linux-backends.md`; `NotificationClicks`; availability probe.
- [ ] T101 [US4] [DESKTOP] On GNOME, KDE Plasma and Cinnamon: the notification shows, a click on its body reaches the app (A-17), « Do not disturb » is respected; steps 20 and 21 of `quickstart.md`.
- [ ] T102 [P] [US4] Portal sink (`ashpd` feature `notification`), chosen when `InstallKind::Flatpak`.
- [ ] T103 [US4] [UI] The unavailable state uses the Linux variant of `NotificationRegistrationFailed`; screenshot scene.
- [ ] T104 [US4] Harness tests with the dbusmock notification daemon: text, hint, click, refusal leaves no « shown » row in the ledger.

## Phase 11: Sign-in, single instance, folders (Lot 11) — US4

**PR 11a** `build(app): start at sign-in and keep one instance on Linux`. **PR 11b** `build(app): name the Linux executable and data folder`. Depend on Lot 3.

- [ ] T110 [US4] `crates/bientot-linux/src/autostart.rs`: desktop-file text and the state table of `contracts/linux-backends.md` (portable, tested with files: absent, hidden, disabled, other path, quoted paths with spaces); `StartupPort` implementation.
- [ ] T111 [US4] `crates/bientot-app/src/launch.rs`: accept `--autostart` as `LaunchIntent::SignIn` on every platform; Windows keeps writing its own flag.
- [ ] T112 [US4] [DESKTOP] Background portal backend (`ashpd` feature `background`), with `StartupPortalReason` **[UI]**; in a Flatpak test build on GNOME and KDE: does the desktop prompt, once (A-18)? Step 22 of `quickstart.md`.
- [ ] T113 [US4] `crates/bientot-linux/src/single_instance.rs`: socket and `flock` implementing `InstanceLock`; tests: second launch hands over, stale socket after `kill -9`, `XDG_RUNTIME_DIR` unset.
- [ ] T114 [US4] [OWNER] Rename the Linux executable to `bientot` (Q11: `[[bin]]` name per target or a second `[[bin]]` entry; Windows keeps `worktime.exe`); data folder `…/bientot` in `launch.rs`; `InstallKind` from the environment.
- [ ] T115 [US4] [UI] Settings and onboarding: startup texts and states for Linux (`contracts/copy.md`), « Open Startup apps » hidden, `StartupMismatchAppImage.Text` with « Corriger »; mockups, screenshot scenes with `--platform linux`.
- [ ] T116 [US4] Harness tests: autostart file written and removed under a temporary `XDG_CONFIG_HOME`; two processes on one data folder.
- [ ] T117 [US4] [UI] Last-resort failure: standard error plus a notification with `NoGraphics.Text` or the existing failure text (`platform::show_fatal`).

## Phase 12: The rest of the integration (Lot 12) — US4, US6

**PR 12a** `build(app): give Linux its own wording`. **PR 12b** `fix(app): read exports from the other platform` (T125; a real fix for Windows users too if A-14 holds: owes a release line). **PR 12c** `feat(app): follow the system language` (T126; ships to Windows: owes a release line and a panel review). **PR 12d** the remainder. Depend on Lot 3; T124 before any **[UI]** task that needs Linux scenes.

- [ ] T120 [US4] [UI] Platform variants in the string table (`crates/bientot-app/strings/*.resw`, `src/strings/table.rs`): a `.Linux` suffix resolved on Linux; the 17 variants of `contracts/copy.md` not already added by their own task; the test of SC-012 (no reachable Linux text contains « Windows » or « Microsoft »; no Windows text contains « Linux »).
- [ ] T121 [P] [US3] Diagnostics on Linux (`SystemInfo`): `/etc/os-release`, architecture, desktop, session type, install kind, backends; shown in Settings › About (`InstallKind.*`) **[UI]** and in the export; test that the export holds no host name or user name.
- [ ] T122 [P] [US4] `wait_for_exit` on Linux (`pidfd_open`, polling fallback) for the restart after import.
- [ ] T123 [US4] [DESKTOP] Reduced motion from the Settings portal; on the Tier 1 desktops, find which key each serves (A-15) and fix it in `contracts/linux-backends.md`; step 25 of `quickstart.md`.
- [ ] T124 [US8] `bientot-screenshots --platform {windows|linux}`: renders the scenes whose text or frame differs; the CI screenshots job uploads both sets; the comparison covers both.
- [ ] T125 [US6] Time zones across platforms: first a test importing a real Windows `.bientot` export on Linux (A-14); if it fails, add the CLDR `windowsZones` mapping to `crates/bientot-app/src/zones.rs` in both directions, with the table's version pinned and a test over every Windows key.
- [ ] T126 [US4] [UI] System language on both platforms: a `SystemLocale` port (`LC_ALL`, `LC_MESSAGES`, `LANG` on Linux; the display languages on Windows) feeding `Language::resolve`; onboarding texts (`OnboardingLanguage*`); spec 004's region default uses the same port.
- [ ] T127 [US4] [DESKTOP] Orca on GNOME and KDE: step 26 of `quickstart.md` (A-16); file each gap with the name Narrator reads.
- [ ] T128 [US4] Plugins on Linux: install by file chooser and by drop on Wayland and X11 **[DESKTOP]**, « Open folder », the revocation-list fetch on its own daily timer when no update check exists (Flatpak, ZIP); step 28 of `quickstart.md`.
- [ ] T129 [US6] Cross-platform data tests: a Windows fixture data folder read on Linux and a Linux one read on Windows give the same week view-models (SC-010); a ledger with Linux origins is read by a Windows build without loss.

**Checkpoint**: US4 and US6 pass from a source build. Everything but packaging is done.

## Phase 13: Flathub (Lot 13) — US5

**PR 13a** `build(app): add the Linux desktop files and the Flatpak manifest`. **PR 13b** `ci(repo): build the Flatpak and open the Flathub update`. Depend on Lots 5–12 and on Q2.

- [ ] T130 [US5] [UI] `packaging/linux/`: desktop entry, AppStream metainfo (FR and EN from `packaging/store/listing.json`, Linux screenshots from T124, release entries generated from `scripts/release-summary/`), icons. The listing text and screenshots go through the panel. `appstreamcli validate` in CI.
- [ ] T131 [US5] [DESKTOP] `packaging/linux/dev.bientot.Bientot.yml` built locally with `flatpak-builder`; on Ubuntu GNOME, Fedora GNOME, KDE and Mint: tray with the watcher name only (A-6), idle, lock, sleep, notifications, autostart, file chooser, GPU rendering. Each failure either gains a permission (justified in `contracts/packaging.md`) or moves a parity-matrix cell to the Flatpak column.
- [ ] T132 [US5] Sandbox switches by `InstallKind::Flatpak`: portal notification and autostart backends, `ksni` without a bus name, no journal, no update source.
- [ ] T133 [US5] `cargo-sources.json` generated from `Cargo.lock`; a CI check fails when it is stale.
- [ ] T134 [US5] `Flatpak` workflow: `flatpak-builder` and `flatpak-builder-lint` on changes to `packaging/linux/` or `Cargo.lock`, and weekly.
- [ ] T135 [US5] Workflow that opens the update pull request on the Flathub repository of the app from a release tag (weekly with the Store submission; manual trigger).
- [ ] T136 [US5] [OWNER] Flathub submission: account, domain verification through `bientot.dev`, the submission pull request with the permission justifications (A-8, A-10). Answer the review; record every permission refused and its consequence.
- [ ] T137 [P] [US5] `docs/flathub-release.md`, the twin of the Store procedure.

## Phase 14: AppImage and self-update (Lot 14) — US5

**PR 14a** `build(app): package the AppImage with Velopack`. **PR 14b** `build(app): update the AppImage from the app`. Depend on Lots 5–12.

- [ ] T140 [US5] [DESKTOP] Velopack end to end by hand on Ubuntu and Fedora, x86_64, then aarch64 if hardware or a runner allows: `vpk pack` two versions, serve the feed locally, the app finds, downloads, replaces `$APPIMAGE` and restarts (A-12). Record the exact file and channel names and correct `contracts/packaging.md`.
- [ ] T141 [US5] Build on the oldest supported glibc: `ubuntu-22.04` runners or a `debian:12` container (A-19); glibc gate (≤ 2.35) in `Rust on Linux`; check the result starts on Ubuntu 22.04 and Debian 12 (containers suffice for « starts »).
- [ ] T142 [US5] `scripts/release-direct.sh --target {win-x64|linux-x64|linux-arm64}`; PNG icon; `vpk download` of the previous Linux release for deltas.
- [ ] T143 [US5] `crates/bientot-app/src/platform/windows/direct.rs` moved out of the Windows backend and compiled on Linux (`velopack` under both targets), behind `UpdateSource`; startup hooks; the update texts are the installer build's (no platform named); `InstallKind::AppImage` only.
- [ ] T144 [US5] [UI] Launcher entry and icons written on first run of the AppImage, « Retirer Bientôt du menu des applications » in Settings › General; mockup, screenshot scene.
- [ ] T145 [US5] `Rust on Linux` builds the `direct` AppImage on every pull request that touches Rust, reports its size, and starts it headless.
- [ ] T146 [US5] aarch64: build on `ubuntu-22.04-arm`; size reported; started under the harness.
- [ ] T147 [US5] [DESKTOP] Step 30 of `quickstart.md` with a real pre-release.

## Phase 15: One release for two platforms (Lot 15) — US5

**PR 15a** `ci(repo): release Windows and Linux together`. **PR 15b** `feat(site): serve the Linux release files`. Depend on Lot 14 and on Q6, Q7.

- [ ] T150 `.github/workflows/release.yml`: `build-linux` jobs (x86_64, aarch64) beside the Windows build; `publish` needs all; draft-then-publish unchanged.
- [ ] T151 Attest every Linux file with `actions/attest`; `Bientot.intoto.jsonl` lists them; `docs/release.md` gains the verification command for an AppImage.
- [ ] T152 [P] `scripts/check-release-summary.sh` accepts the optional `platforms:` line; tests.
- [ ] T153 [P] `scripts/release-notes.sh` prints « Windows » and « Linux » sections; `store-release-notes.sh` drops Linux-only lines; the metainfo generator of T130 drops Windows-only lines.
- [ ] T154 `services/bientot/src/releases.ts`: Linux feeds, packages and AppImages under `/updates/` and `/download/linux*`; tests. Deployed before the first release that carries Linux files.
- [ ] T155 Until T186, Linux files are attached to releases and labelled « preview » in the notes; the site does not show them.

## Phase 16: Usage signal (Lot 16) — US5

**PR 16a** `docs(repo): describe the usage signal on Linux`. **PR 16b** `feat(site): accept the Linux channels`. **PR 16c** `build(app): send the usage signal from Linux`. Depend on Q3; 16b before 16c.

- [ ] T160 [OWNER] `docs/privacy.md` and the site's privacy page (FR, EN): the new channels, and the two keys if Q3 is accepted; panel review of the wording **[UI]**.
- [ ] T161 `services/bientot/src/ping.ts`, `store.ts`, `stats.ts`: channels `flatpak` and `appimage`; the optional key pair per `contracts/usage-signal.md`.
- [ ] T162 The app's channel from `InstallKind`; the two keys if accepted; a Windows body stays byte-identical.
- [ ] T163 Statistics page and the PostHog saved query (`usage_daily` version 2): platform, and desktop and session if accepted, under the existing suppression.
- [ ] T164 Tests of `contracts/usage-signal.md`.

## Phase 17: The site (Lot 17) — US5

**PR 17** `feat(site): offer Bientôt for Linux`. Depends on Lot 15.

- [ ] T170 `/download` by `User-Agent`; `/download/windows`; the Linux section renders only when the latest release holds Linux files and the launch flag is set.
- [ ] T171 [UI] Download section for Linux (Flathub first, AppImage second, architecture choice, Windows as a secondary link and the reverse on Windows), FR and EN; mockup of the page; panel review.
- [ ] T172 [UI] FAQ « Mac ou Linux ? », the « pour Windows » mentions, JSON-LD `operatingSystem`, meta descriptions.
- [ ] T173 [UI] Help page « Afficher l’icône de Bientôt » (GNOME without a tray: what the extension is, how to add it, what window mode does), the target of `WindowModeNotice.Link`.
- [ ] T174 Tests of the routes and of the rendered sections.

## Phase 18: Documents, full validation, launch (Lot 18) — all stories

**PR 18a** `docs(repo): document Bientôt on two platforms`. **PR 18b** `feat(app): ship Bientôt for Linux`.

- [ ] T180 [P] `README.md`: one product, two platforms; install for each; build from source on each.
- [ ] T181 [P] `CONTRIBUTING.md` and `docs/architecture.md`: the platform crates, the ports, the composition root, the harness, checking the other platform locally.
- [ ] T182 [P] `docs/*.md` that describe Windows procedures or spell Windows paths (plugins, diagnostics, startup registration, data folder): a Linux section or neutral wording.
- [ ] T183 `docs/platforms.md` final (tiers, parity matrix as shipped, known limits); `docs/privacy.md` lists the Linux files outside the data folder (`data-model.md`).
- [ ] T184 [DESKTOP] The full 30 steps of `quickstart.md` on the four Tier 1 desktops, for the Flatpak and the AppImage, x86_64; the Windows pass; results in `research.md`. Every open assumption A-1 to A-20 is marked verified or replaced.
- [ ] T185 Record start time and memory on the reference Linux machine beside the Windows figures in `docs/ci-quality-gates.md`; SC-005 day-long comparison on one desktop per session type.
- [ ] T186 [UI] [OWNER] Launch: set the site flag, remove « preview », `scripts/release-summary/` file (« Bientôt est disponible pour Linux… »), Flathub listing live, announcement; reviewed by the panel.

## Dependencies

```text
Lot 0 ──────────────┬──────────────────────────────► Lot 9 (also Q4)
Lot 1 ─► Lot 2 ─► Lot 3 ─┬─► Lot 4 ─► Lot 5 ─► Lot 6 ─┐
   (Lot 0 before Lot 2)  ├─► Lot 7 ───────────────────┤
                         ├─► Lot 8 ─► Lot 9 ──────────┤
                         ├─► Lot 10 ──────────────────┼─► Lot 13 (Q2) ─┐
                         ├─► Lot 11 ──────────────────┤                ├─► Lot 18
                         └─► Lot 12 ──────────────────┴─► Lot 14 ─► Lot 15 ─► Lot 17 ─┘
                                                              Lot 16 (Q3) ─────────────┘
```

- Lots 4, 7, 8, 10, 11 and 12 can run in parallel once Lot 3 is merged.
- Lot 9 waits for the owner (Q4); nothing else waits for Lot 9 except the
  launch.
- Lot 16b (service) deploys before any Linux build sends a signal, and
  Lot 15b before any release carries Linux files.

## Tasks that change what the user sees or reads

Every one goes through the three-reviewer panel and updates the mockups and
the design system: T044, T053, T064, T075, T084, T094, T103, T112, T115,
T117, T120, T121, T126, T130, T144, T160, T171, T172, T173, T186.

## Tasks that need a real Linux desktop

T041, T042, T043, T044, T055, T066, T072, T077, T082, T086, T091, T095,
T101, T112, T123, T127, T128, T131, T140, T147, T184; and T026 on Windows.
Hardware rather than a virtual machine: T086 (suspend), a two-monitor setup
for T042 and T066, aarch64 for T140 and T146 when no runner suffices.
