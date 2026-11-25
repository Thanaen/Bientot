# Implementation Plan: Linux support

**Branch**: `005-linux-support` | **Date**: 2026-11-24 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/005-linux-support/spec.md`

## Summary

Bientôt becomes one product on two platforms of equal standing. The work has
two halves, in this order.

**First, on Windows, with no behaviour change.** Everything the app asks of
the operating system goes behind ports. A new crate, `bientot-platform`,
holds the ports and the logic that was never Windows-specific but lives in
`bientot-windows` today (the activity source, the lifecycle recorder, the
pairing and recovery of power and session history, the update service).
`bientot-windows` keeps the Win32 and WinRT backends only. `bientot-app`
receives one `Platform` value from one composition root and contains no
platform condition anywhere else; a script in CI enforces that.

**Then, on Linux, behind the same ports.** A new crate, `bientot-linux`,
implements them: StatusNotifierItem tray (`ksni`), flyout placement per
session type, four idle backends chosen at run time, logind for lock and
sleep, the journal for power history, desktop notifications, XDG autostart,
a Unix socket for the single instance. Two packages ship: a Flatpak on
Flathub (the counterpart of the Microsoft Store build) and an AppImage with
Velopack (the counterpart of `Setup.exe`), for x86_64 and aarch64.

Delivery is nineteen lots (`lots.md`), each a pull request or a short series
that leaves Windows shippable. The order removes risk first: extraction,
Linux in CI, a tray and flyout spike on real desktops, then idle and
session, then everything else, then packaging and the launch.

## Technical Context

**Language/Version**: Rust 1.97, edition 2024 (`rust-toolchain.toml`). No
toolchain change: the Linux host target is installed by default.

**Primary Dependencies**: existing `gpui-pre` 0.3.9 (X11 and Wayland
backends on by default), `jiff`, `rusqlite`, `ureq`, `velopack` 1.2.161,
`wasmi`. New, Linux only: `ksni` 0.3 (tray). Already in `Cargo.lock` through
GPUI and used directly from now on: `zbus` 5, `ashpd` 0.13 (features
`background`, `notification`, `settings` added), `notify-rust` 4,
`wayland-client` and `wayland-protocols` (`staging`), `x11rb` (features
`randr`, `screensaver` added), `rustls` 0.23. `ureq` switches from
`native-tls` to `rustls` on Linux only (`research.md`, R12).

**Storage**: unchanged formats. `settings.json`, `worktime.db`,
`app-state.json`, `logs/`, `plugins/` under one data folder:
`$XDG_DATA_HOME/bientot` on Linux. `app-state.json` gains two optional keys
on Linux (`data-model.md`). No ledger migration. New ledger origin strings
for Linux evidence (`data-model.md`). The `.bientot` transfer file moves
between platforms in both directions.

**Testing**: `cargo test --workspace` on Ubuntu and Windows as today. New:
`bientot-linux` unit tests on any OS for its portable half; integration
tests under a headless desktop harness (`Xvfb` with `openbox`, `sway`
headless, `dbus-run-session` with `python-dbusmock`, Mesa software Vulkan);
golden-ledger tests that feed one timeline of input, lock and sleep to the
Windows decoder and to the Linux decoder and require identical ledger rows.
Real desktops are validated by hand with `quickstart.md`.

**Target Platform**: Windows 10 22H2 and Windows 11, x64 (unchanged).
Linux with glibc 2.35 or later, x86_64 and aarch64, X11 or Wayland
(`spec.md`, « Supported desktops »).

**Project Type**: desktop application, Cargo workspace.

**Performance Goals**: same as Windows. Flyout visible under 1 s from a
click on a warm start; ready file under 1 s from process start; under 60 MB
resident at rest. Reported, not enforced, on hosted runners (no GPU).

**Constraints**: stripped executable of the `release` and `direct` builds
under 20 MB on each platform, enforced in CI (`research.md`, R13 « Size
budget »). On Linux, `Rust on Linux` reports the size and warns above 20 MB
until the Linux size task, issue #226, brings the executable under; it then
fails the job, as on Windows. No elevated privilege, no membership of the `input` group, no
read of `/dev/input` (Principle I). No GTK or Qt at run time.

**Scale/Scope**: about 2 400 lines of Win32 code have a Linux counterpart to
write (`research.md`, R2); about 7 000 lines move between crates without
change; 17 strings per language gain a Linux variant; two packages; three
workflows; one site section.

## Constitution Check

Checked against constitution 1.2.0, then against the 1.3.0 proposed below.

| Principle | Check | Result |
| --- | --- | --- |
| I. Local data and privacy | Linux reads input **idle time** only, from the compositor or the desktop, never events or devices. Journal and logind are read locally. No new data leaves the machine unless the owner accepts Q3; the usage signal's fields stay a closed list written in `docs/privacy.md` | Pass, Q3 open |
| II. Truthful accounting | Every Linux gap is shown as unknown, never guessed: no idle backend, no lock report, no readable journal. The « Avant Bientôt » rebuild on Linux cannot know past locks; it is an accounting decision (Q4, D-026) taken before Lot 9 | Pass, Q4 open |
| III. Tray-first experience | The tray is the primary surface on Linux too. « Windows system tray » in the principle's text is amended. Where a desktop has no tray host, window mode is a stated fallback (FR-012), not a second experience | Pass with amendment |
| IV. Performance budget | Same budgets, measured on Linux by the same kind of script | Pass |
| V. Simple modular design | « Windows and .NET platform libraries » in the text is amended. One new port crate and one backend crate per platform; each new dependency is justified in `research.md`. No abstraction is added for a single caller: every port has two implementations | Pass with amendment |
| VI. Plugin extensions | The plugin runtime is pure Rust and unchanged; its tests and hostile corpus run on both platforms | Pass |
| Constraints: « Initial platform support is limited to… Windows » | Amended | Pass with amendment |
| Workflow rule 2 (accounting changes need a recorded decision) | D-026 proposed below | Pass once ratified |

No violation needs a justification in « Complexity Tracking » beyond the two
new crates.

### Proposed amendment: constitution 1.3.0

Applied by task T002 after the owner ratifies it (open question Q9). Minor
version: one principle added, three texts widened, nothing removed.

1. **Principle III**, replace « the Windows system tray » with « the system
   tray of the desktop (the notification area on Windows, a
   StatusNotifierItem host on Linux) », and add: « Where a Linux desktop has
   no tray host, Bientôt says so and runs in a window; that fallback is
   never the default on a desktop that has one. »
2. **Principle V**, replace « Windows and .NET platform libraries » with
   « the libraries of the platform and of the Rust ecosystem already in the
   dependency tree ».
3. **Product and quality constraints**, replace the sentence that limits
   initial support to Windows with: « Bientôt supports Windows and Linux.
   The supported versions, distributions and desktops are listed in
   `docs/platforms.md` and change by pull request. »
4. **New Principle VII, Platform parity.**

   > Windows and Linux are equal. A feature is designed, specified, built
   > and tested for both before it ships on either, unless the platform
   > cannot provide it; that case is written in the parity matrix of
   > `docs/platforms.md` with what the user sees instead.
   >
   > - Every pull request compiles, lints and tests both platforms. Neither
   >   set of checks is optional for a change to shared code.
   > - Platform-specific code lives in the platform crates and in the one
   >   composition root of the app. A platform condition anywhere else is a
   >   defect; CI refuses it.
   > - A specification states the behaviour on each platform. « Same » is a
   >   claim that a test or a validation step backs.
   > - Accounting rules are identical. Where the evidence a platform can
   >   give differs, the difference is a recorded product decision and is
   >   visible to the user.
   > - Wording never names the other platform.

### Proposed decision: D-026

Added to `docs/product-decisions.md` by task T003 once Q4 is answered. Text
for the recommended answer:

> **D-026 — Evidence on Linux.** The accounting rules of Linux are those of
> Windows. Evidence comes from the desktop: input idle time from the
> compositor or the desktop shell, lock and unlock from logind's
> `LockedHint`, sleep and resume from logind, past sleep from the journal
> when the user may read it.
>
> Linux keeps no record of past locks. The first-start rebuild (« Avant
> Bientôt ») therefore covers the time from the opening of the session to
> the first start, minus known sleep, as one provisional review whose text
> says that locked time is not known. It is never credited without the
> review. When the session opening is not known, nothing is rebuilt.
>
> When a desktop reports no idle time, nothing is counted automatically.
> When it reports no lock state, the session state is unknown and the day
> is accounted from activity alone. Both are shown.

## Project Structure

### Documentation (this feature)

```text
specs/005-linux-support/
├── spec.md              # stories, requirements, parity matrix, supported desktops
├── plan.md              # this file
├── research.md          # audit, GPUI verification, decisions, assumptions
├── data-model.md        # ports' data, new settings and ledger origins
├── quickstart.md        # manual validation on a real desktop
├── tasks.md             # tasks by lot
├── lots.md              # one row per GitHub issue, open questions
├── checklists/requirements.md
└── contracts/
    ├── platform-ports.md   # Rust signatures of every port
    ├── linux-backends.md   # D-Bus names, protocols, selection order
    ├── packaging.md        # Flatpak, AppImage, file names, feeds, CI gates
    ├── usage-signal.md     # the request body, both platforms
    └── copy.md             # FR and EN texts that differ on Linux
```

### Source Code (repository root)

```text
crates/
├── bientot-domain/        # unchanged
├── bientot-core/          # unchanged (NotificationSink, FinalPaths stay)
├── bientot-store/         # unchanged
├── bientot-platform/      # NEW, any OS: ports + portable platform logic
│   └── src/
│       ├── lib.rs             # PowerState, dotnet_ticks, InstallKind
│       ├── activity.rs        # LastInputProbe, BuiltInActivitySource (moved)
│       ├── lifecycle.rs       # LifecycleChange, -Transition, -EventRecorder,
│       │                      # LifecycleHooks (moved); LifecycleSource (new)
│       ├── power_history.rs   # pairing, recording (moved); PowerBoundaryLog (new)
│       ├── session_history.rs # recovery (moved); SessionBoundaryLog (new)
│       ├── startup.rs         # StartupRegistrationState, LaunchIntent,
│       │                      # StartupPort (moved from the app)
│       ├── single_instance.rs # Acquired, InstanceLock (new)
│       ├── notifications.rs   # NotificationRequest, text building (moved)
│       ├── updates.rs         # UpdateService and its traits (moved, renamed)
│       ├── tray.rs            # MenuEntry (moved from the app); Tray (new)
│       ├── placement.rs       # FlyoutPlacement, Desktop (new)
│       ├── motion.rs          # ReducedMotion (new)
│       └── system.rs          # SystemInfo (new)
├── bientot-windows/       # Win32 and WinRT backends only
├── bientot-linux/         # NEW: portable half on any OS, `native` on Linux
│   └── src/
│       ├── tray.rs            # MenuEntry → ksni menu; host state
│       ├── placement.rs       # work area maths; x11.rs, wayland.rs
│       ├── idle.rs            # selection; idle/{ext_idle,mutter,screensaver,xss}.rs
│       ├── logind.rs          # property changes → LifecycleChange
│       ├── journal.rs         # journal JSON → power boundaries
│       ├── autostart.rs       # desktop file text and states; portal.rs
│       ├── single_instance.rs # socket and lock
│       ├── notifications.rs   # notify-rust and portal sinks
│       ├── motion.rs, system.rs
│       └── examples/shell-spike.rs   # Lot 4, never shipped
├── bientot-app/
│   └── src/
│       ├── platform/
│       │   ├── mod.rs         # struct Platform, fn build, null platform
│       │   ├── windows.rs     # today's `mod native` of main.rs
│       │   ├── windows/tray.rs # Tray and Desktop over `tray-icon`, DWM
│       │   └── linux.rs
│       └── …                  # no platform condition outside platform/
└── bientot-plugin*/       # unchanged

packaging/
├── msix/, store/          # unchanged
└── linux/                 # NEW: desktop entry, metainfo, icons, Flatpak manifest

scripts/
├── check-platform-cfg.sh  # NEW gate
├── platform-cfg-allowlist.txt
├── linux-desktop-harness.sh   # NEW: headless X11, Wayland, D-Bus mocks
├── measure-performance.sh     # NEW twin of measure-performance.ps1
└── release-direct.sh      # gains a target argument
```

**Structure Decision**: three platform crates rather than `cfg` modules in
one. `bientot-windows` already proves the pattern (a portable half tested on
any OS, a `native` half behind one `cfg`); naming the shared part
`bientot-platform` stops Linux code from importing a crate called
« windows », and lets `bientot-app` depend on the ports alone.

## Architecture

### Ports

A port is a trait in `bientot-platform` with exactly two production
implementations (Windows, Linux) and one in-memory implementation for
tests. `contracts/platform-ports.md` gives the signatures. Summary:

| Port | Exists today as | Windows backend | Linux backend |
| --- | --- | --- | --- |
| `LastInputProbe` | same trait, in `bientot-windows::activity` | `SystemLastInput` | `IdleProbe` over the selected backend |
| `LifecycleSource` | `lifecycle::native` functions called from `main.rs` | message-only window | logind thread |
| `PowerBoundaryLog` | `SystemEventLog` (returns Windows records) | System log reader + its parser | journal reader; boot-time fallback |
| `SessionBoundaryLog` | `WinlogonEventLog` (returns Windows records) | Winlogon log reader + its parser | logind session opening |
| `NotificationSink` | same trait, in `bientot-core` | toast sink | `notify-rust` or portal sink |
| `StartupPort` | same trait, in `bientot-app::settings::startup` | `Run` key or MSIX task | autostart file or Background portal |
| `InstanceLock` | functions of `single_instance::native` | named pipe | Unix socket and `flock` |
| `UpdateSource` | `UpdateStore` | Store, Velopack | Velopack (AppImage); none in Flatpak |
| `Tray` | functions of `bientot-app::tray` | `tray-icon` | `ksni` |
| `Desktop` | `tray::work_area`, `clicked_monitor`, `place_above_taskbar` | monitor work area; `platform::place_window` with `SetWindowPos`, DWM | `x11rb`, layer-shell, or compositor |
| `ReducedMotion` | `animation_effects` | `SPI_GETCLIENTAREAANIMATION` | Settings portal |
| `SystemInfo` | `diagnostics`, `has_package_identity` | Win32 | `/etc/os-release`, environment |
| `FinalPaths` | same trait, in `bientot-core` | Win32 | `NoFinalPathCheck` |

The two history ports move **one level up** from where the traits sit
today: they return boundaries (`boot`, `shutdown`, `suspend`, `resume`,
`logon`, `lock`, …), not Event Log records. The Windows parsers stay in
`bientot-windows`, behind the port; pairing, idempotent recording and the
« Avant Bientôt » recovery are the shared code above it.

`LastInputProbe` and `LastInputReading { tick_count, last_input_tick }`
keep their shape: the domain's `IdleStateTracker` is accounting code and
must not move for this feature. A Linux probe converts « monotonic uptime,
idle for d » into the same pair (`reading_from_idle`), so one tracker
serves both platforms and the golden-ledger tests can compare them.

### Composition root

```rust
// crates/bientot-app/src/platform/mod.rs
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(any(windows, target_os = "linux")), path = "null.rs")]
mod backend;

pub use backend::build;   // fn build(ctx: &PlatformContext) -> anyhow::Result<Platform>
```

`Platform` is a struct of boxed ports (`contracts/platform-ports.md`).
`main.rs` calls `platform::build` once and hands the parts to the host,
shell, settings and updates, which take `&dyn Port` or a generic. Tests and
the screenshot binary build `Platform::null()`. Any other operating system
gets the null platform, as it gets no-ops today, so `cargo test` keeps
working there.

### No scattered conditions

Rule (constitution VII once ratified, `CONTRIBUTING.md`): `cfg(windows)`,
`cfg(unix)`, `cfg(target_os …)`, `cfg(target_family …)` and their `cfg!`
and `cfg_attr` forms may appear only in:

- `crates/bientot-windows/`, `crates/bientot-linux/`;
- `crates/bientot-app/src/platform/`;
- `Cargo.toml` target tables and `build.rs` files;
- the lines listed in `scripts/platform-cfg-allowlist.txt`, each with a
  reason. Initial content: the Unix branch of
  `bientot-core/src/diagnostics/export.rs`, the DPAPI module of
  `bientot-store`, the memory measure of the hostile-corpus example, the
  `windows_subsystem` attribute of `crates/bientot-app/src/main.rs`, which
  only the crate root can carry.

`scripts/check-platform-cfg.sh` greps `crates/` and fails on anything else.
It runs in the `Rust` job, so it gates every pull request.

### The compiler stays honest

| Check | Where | When |
| --- | --- | --- |
| Lint and test the workspace for Linux | `Rust` (Ubuntu) | every pull request, as today |
| Lint the workspace **for the Windows target** (`cargo clippy --target x86_64-pc-windows-msvc`, clang and `llvm-lib` as `CONTRIBUTING.md` describes) | `Rust` (Ubuntu) | every pull request, new |
| Platform-condition gate | `Rust` (Ubuntu) | every pull request, new |
| Tests, release build, size, start, MSIX, Velopack on Windows | `Rust on Windows` | as today |
| Release build, size, glibc, hostile corpus, headless start, desktop harness on Linux | `Rust on Linux` | same path filter as `Rust on Windows`, new |

So a pull request that only edits Linux code is still type-checked for
Windows, and the reverse, before any platform job starts.

### Moving Windows code without changing behaviour

Lots 1 and 2 are refactors. Their proof:

- No test is edited except for its `use` lines. Tests move with their
  code.
- The ledger written by the lifecycle, power-history and session-history
  tests is byte-identical before and after (existing fixture tests).
- `bientot-screenshots` output is identical (the CI comparison that
  exists).
- `verify-package-*.ps1`, `verify-startup-visibility.ps1` and
  `test-rust-toast-activation.ps1` pass on the Windows job.
- The Windows executable size changes by less than 50 KB.
- One manual pass of the Windows section of `quickstart.md` before Lot 2
  merges (tray, flyout placement on two monitors, toast click, autostart).

Names that carry « Windows » in shared code are renamed in Lot 1 where the
compiler finds every use (`LaunchIntent::WindowsStartup` →
`LaunchIntent::SignIn`); the command-line flag `--worktime-windows-startup`
is data that installed copies depend on and stays.

## Conventions for the pull requests

- **Titles.** Conventional Commits with the scopes of `pr-title.yml`:
  `app`, `site`, `repo`. Until the launch, pull requests that add Linux
  code are `build(app)`, `refactor(app)`, `test(app)`, `ci(repo)` or
  `docs(repo)`: no Windows user sees a change, so no release line is owed
  and the weekly notes stay true. The launch is one `feat(app)` (T186).
- **Release summary.** Any `feat(app)`, `fix(app)` or `perf(app)` pull
  request adds its file under `scripts/release-summary/` with one `fr:`
  and one `en:` line. From Lot 15 a file may start with `platforms:
  windows` or `platforms: linux`.
- **Size.** Every lot that adds a dependency states the Linux executable
  size before and after in its description; `Rust on Linux` reports it
  against 20 MB from Lot 3, and fails above it once the Linux size task,
  issue #226, lands.
- **UI and wording.** Tasks marked **[UI]** go through the three-reviewer
  panel (designer critique, UX writing, screenshot review) and update the
  GPUI mockups and the design system in the same pull request.
- **Real desktops.** Tasks marked **[DESKTOP]** cannot be proven by CI.
  Each names the desktops and the observation; results are appended to
  `research.md` under « Validation results » with date, distribution,
  desktop version and session type.
- **Issues.** One epic, one issue per lot (`lots.md`), labels `linux` and
  the lot's area, milestone « Linux », kept in step with `tasks.md` by the
  pull request that changes either.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
| --- | --- | --- |
| Crate `bientot-platform` | Linux needs the 7 000 portable lines that sit in `bientot-windows` | Depending on `bientot-windows` from Linux code works today but hides the rule « shared code names no platform » and keeps Win32-shaped ports (`SystemEventLog`) as the seam |
| Crate `bientot-linux` | About 2 500 lines of D-Bus, Wayland and X11 code | `cfg` modules inside `bientot-app` would scatter conditions and make the gate impossible |
| Four idle backends | No single interface exists on Linux (`research.md`, R6) | Any one of them leaves a Tier 1 desktop without activity detection |
| Two packages | The sandbox costs journal history and self-update; the AppImage costs store presence | One package leaves either Flatpak-only distributions or non-Flatpak users without an official build |
