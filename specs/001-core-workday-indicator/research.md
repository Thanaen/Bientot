# Research: Core Workday Indicator

**Status**: Complete  
**Date**: 2026-07-27

## R-001 — WinUI 3 and system tray

**Decision**: Use WinUI 3 / Windows App SDK for user-facing UI and isolate tray
integration behind `ITrayIconService`.

**Rationale**: WinUI 3 is the selected modern Windows UI stack and aligns with
MSIX, Windows notifications, localization, and accessibility. It does not
provide `NotifyIcon`, so the first implementation uses the stable MIT-licensed
`H.NotifyIcon.WinUI` package. A short spike must validate flyout positioning,
Explorer restart recovery, DPI, theme, keyboard access, and packaged/unpackaged
lifecycle before the rest of the UI depends on it.

**Alternatives considered**:

- Windows Forms: simplest native `NotifyIcon`, rejected in favour of WinUI 3.
- WPF: mature desktop stack, but not the desired UI framework.
- Direct `Shell_NotifyIcon`: fewer dependencies but more interop and lifecycle
  code; retained as fallback behind the abstraction.

## R-002 — ActivityWatch compatibility

**Decision**: Treat `aw-server-rust` as the primary tested backend and support
Python `aw-server` using a narrow read-only REST contract.

**Rationale**: Both implementations expose the bucket/event API required by
WorkTime. Raw bounded AFK event reads are easy to test and avoid depending on
the query language or the API explorer, which `aw-server-rust` does not provide.
Capability probing through `/api/0/info` is safer than process detection.

`aw-server-rust` can optionally protect API endpoints with bearer
authentication. WorkTime handles `401` by requesting a key and protecting it
with current-user DPAPI.

**Revised 2026-10-11** (D-021, R-009): ActivityWatch is an optional source.
This compatibility decision applies only when the user selects it.

## R-003 — Local persistence

**Decision**: Use SQLite for WorkTime-owned events, adjustments, credit, and
notification receipts; use JSON for preferences.

**Rationale**: Manual edits and retroactive weekly credit require
transactionality, ordering, migrations, and deterministic recomputation.
ActivityWatch remains read-only and can be unavailable without losing Windows
session transitions.

**Revised 2026-10-11** (D-021, R-009): the ledger also stores the built-in
source's activity spans and the activity source selections.

## R-004 — Time representation

**Decision**: Store instants as UTC and keep the relevant Windows timezone ID
with workday-derived records. Use `DateTimeOffset`, `TimeZoneInfo`, and injected
`TimeProvider`; do not add a time library initially.

**Rationale**: The domain needs deterministic instants, local workday
boundaries, and DST tests, but only one machine timezone in v1. Add a dedicated
time library only if native APIs make an identified rule unsafe or unclear.

## R-005 — Packaging

**Decision**: Publish portable self-contained x64 ZIPs for alpha feedback and a
signed x64 MSIX for public v1.

**Rationale**: ZIPs minimize alpha friction. MSIX provides the normative package
identity and integrates cleanly with startup registration, notifications,
install/uninstall, and signing. Portable limitations are documented rather than
emulated with product-specific branching in the domain.

**Revised 2026-10-11** (#108, D-016): public v1 ships as an x64 MSIX through the
Microsoft Store, which signs it after certification; the project buys no
code-signing certificate. Unsigned portable ZIPs stay on GitHub Releases for
alphas.

## R-006 — Updates

**Decision**: Query GitHub Releases metadata at most once per day when enabled,
show an offer, and require user action to install.

**Rationale**: This is predictable and privacy-preserving. No updater service or
silent mutation is justified for v1.

**Revised 2026-10-11** (#108, D-016): WorkTime no longer queries GitHub
Releases. The Store-installed build asks the Microsoft Store through
`StoreContext` at most once per day when enabled, offers the update, and
installs it only on user action; the Store's own automatic updates follow the
user's Store settings. The portable build has no update check.

## R-007 — Localization and accessibility

**Decision**: Ship French and English `.resw` resources and convey every colour
state with text and iconography.

**Rationale**: Resource-first UI avoids an expensive later retrofit. Colour
alone is insufficient for accessibility and violates the project constitution.

## R-008 — Diagnostics

**Decision**: Keep redacted local rolling logs capped at 20 MiB and 14 days.
Export only through explicit user action.

**Rationale**: Local diagnostics are sufficient for a local-first desktop app.
Telemetry and automatic crash uploads would add a privacy and operations surface
without being necessary for the core product.

## R-009 — Built-in idle detection

**Decision**: Make a built-in activity source the default: poll
`GetLastInputInfo` every 5 seconds, switch to idle after 180 seconds without
input (configurable), and store `active`/`idle` spans in the SQLite ledger.
Keep ActivityWatch as an optional source chosen in settings, never combined
with the built-in source for the same period.

**Rationale**: WorkTime reads only the AFK status from ActivityWatch, and
`aw-watcher-afk` derives it on Windows from `GetLastInputInfo` polled every 5
seconds with a 180-second threshold. Reading the same signal directly gives
the same accuracy without a local HTTP server, port, bucket choice, or a
second application to install. It stores only timestamps, which is easier to
explain than a tracker that also records window titles. `GetLastInputInfo` has
been a stable Win32 API since Windows 2000 and adds no package dependency. The
one loss is activity from before WorkTime starts, which only a source running
independently of WorkTime can provide; users who need it keep ActivityWatch.

**Alternatives considered**:

- Install ActivityWatch automatically through winget, or bundle
  `aw-server-rust` and `aw-watcher-afk`: rejected. It adds a second tray
  application and a local server to supervise, MPL-2.0 redistribution
  tracking, likely antivirus false positives for packaged Python executables,
  a Store-unfriendly third-party installer, and window-title tracking WorkTime
  does not need. It also contradicts FR-037.
- Combine both sources, for example ActivityWatch before launch and built-in
  after: rejected, because it needs merge rules for overlapping evidence.
- Fall back automatically from a failing ActivityWatch to the built-in source:
  rejected for the same reason; the failure is shown instead.

Source: the ActivityWatch study of 2026-10-11 and the `aw-watcher-afk`
sources (`aw_watcher_afk/windows.py`, `aw_watcher_afk/config.py`).

## Dependency policy

Use central package management and pin exact versions in the repository.
Dependencies must be latest stable versions compatible with the selected .NET
and Windows App SDK at implementation time. Preview packages require a written
exception. Direct dependencies currently justified are:

- Windows App SDK / WinUI 3;
- CommunityToolkit.Mvvm;
- H.NotifyIcon.WinUI;
- Microsoft.Data.Sqlite;
- System.Security.Cryptography.ProtectedData if not included by the target;
- FsCheck.Xunit for randomized interval invariants;
- xUnit and test tooling.
