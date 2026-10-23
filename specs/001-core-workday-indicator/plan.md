# Implementation Plan: Core Workday Indicator

**Status**: Ratified
**Spec**: [spec.md](spec.md)  
**Prepared**: 2026-07-27

## Technical direction

Use a small native Windows desktop process with a deterministic calculation
library and explicit adapters around the activity sources, the Windows session
APIs, the clock, persistence, and tray UI.

Revised on 2026-10-11 (D-021): the default activity source is built into
WorkTime and polls `GetLastInputInfo`. ActivityWatch is an optional source the
user selects in settings. Exactly one source applies to any instant.

### Stack

- C# 14 on .NET 10 LTS (latest stable LTS baseline at planning time)
- WinUI 3 on Windows App SDK for the flyout, onboarding, history, and settings
- `H.NotifyIcon.WinUI` behind an internal tray abstraction
- CommunityToolkit.Mvvm for presentation models and commands
- `GetLastInputInfo` and `GetTickCount` (Win32) for the built-in activity
  source
- `HttpClient` plus small typed DTOs for the optional ActivityWatch localhost
  API
- `System.Text.Json` for low-volume user configuration
- SQLite via `Microsoft.Data.Sqlite` for WorkTime-owned events and derived ledger
- Windows DPAPI scoped to the current user for an optional ActivityWatch API key
- xUnit for the calculation core and adapter contract tests
- GitHub Actions for build, test, packaging, and release
- Windows 11 x64 only for v1

WinUI 3 is the selected UI framework. Because it has no native system-tray
component, the tray package is isolated so it can be replaced by direct
`Shell_NotifyIcon` interop without touching domain or view-model code.

## Architecture

```text
Windows session notifications ─┐
Built-in input polling ─────────┤
Manual controls/corrections ────┼─> local event ledger ─┐
                                │                        │
ActivityWatch localhost API ────┴─> source adapters ─────┤
  (optional, when selected)                              v
Clock + configuration + credit policy ─────────> accounting engine
                                                         │
                                                         v
                                            immutable daily summary
                                              │         │
                                              v         v
                                      tray/notifications  detail/settings
```

### Components

1. **WorkTime.Domain**
   - interval normalization and splitting;
   - accounting policy;
   - projection and status thresholds;
   - source quality and degraded-state rules;
   - no UI, HTTP, Windows, or system-clock dependencies.

2. **WorkTime.ActivityWatch** (optional source, D-021)
   - capability discovery through `GET /api/0/info`;
   - bucket selection;
   - AFK event retrieval;
   - optional bearer authentication;
   - `aw-server-rust` and Python `aw-server` contract fixtures;
   - DTO-to-domain normalization;
   - compatibility diagnostics.

3. **WorkTime.Windows**
   - `GetLastInputInfo` adapter for the built-in activity source, behind a
     port so the 5-second sampling, the idle threshold, and the active/idle
     transitions stay in tested services and domain code;
   - lock/unlock and power/session notifications;
   - current session-state detection at startup;
   - startup registration;
   - tray icon and Windows notifications.

4. **WorkTime.App**
   - refresh orchestration and cancellation;
   - settings persistence;
   - summary caching;
   - notification scheduling and deduplication;
   - daily Microsoft Store update check (packaged build only);
   - WinUI views and flyouts;
   - presentation models;
   - localized resources;
   - structured local diagnostics, redaction, and explicit export.

5. **WorkTime.Persistence**
   - SQLite schema and migrations;
   - session transitions and tracking-state markers;
   - built-in activity spans and activity source selections;
   - append-only manual adjustments;
   - cross-day credit entries and deterministic recomputation;
   - notification receipts.

## Local event ledger

WorkTime records its own state in SQLite and treats ActivityWatch as read-only:

- Windows session notifications append `locked` or `unlocked` transitions;
- while the built-in source is selected, it appends an activity span at each
  start and each `active`/`idle` transition, and advances the open span's
  settled instant, which for an `active` span is the last input read (see the
  [built-in activity source contract](contracts/built-in-activity-source.md));
- changing the activity source appends a selection with its effective instant;
- startup records the current state and reconciles any open interval;
- pause, resume, and finish-day append control markers;
- corrections and manual work append adjustments rather than rewriting source
  history;
- credit entries are derived from daily summaries and can be recomputed;
- notification receipts enforce once-per-trigger-per-day delivery.

SQLite is warranted by the now-required correction, review, cross-day credit,
and notification features. It also preserves session transitions while
ActivityWatch is temporarily unavailable and avoids coupling WorkTime writes to
an evolving external API.

## Activity sources

Two implementations of `IActivitySourcePort` exist. The built-in source is the
default; ActivityWatch is optional. The selection history in the ledger says
which one answers for each instant, and a read that spans a switch is split at
the switch: each part goes to its own source, and the results are never merged
for the same period. Each part's coverage is clamped to that part, because a
source can answer with more than it was asked for (a retained ActivityWatch
snapshot during backoff). WorkTime does not fall back from a failing source to
the other one; the failure is shown as a degraded state.

Before the first selection row, no source applies and activity is unknown.
An upgraded installation gets an explicit `activitywatch` row effective at the
Unix epoch on its first start; a new installation gets its first row at
onboarding confirmation, so time before confirmation is unknown and
ActivityWatch is never read for it (see the
[data model](data-model.md#activitysourceselection)).

Bucket choice (`SelectionRequired`) is specific to ActivityWatch. The built-in
source never needs a choice, so the port's result types stop requiring a bucket
identifier for a snapshot. When the choice blocks only the ActivityWatch part
of a read that spans a switch, `SelectionRequired` carries the other parts'
coverage; that evidence is kept and only the blocked time is unknown.

## ActivityWatch compatibility contract

This section applies only when ActivityWatch is the selected source. WorkTime
uses the smallest read-only API surface common to the two supported server
implementations:

- `GET /api/0/info` for reachability, version, implementation hint, hostname,
  and device identity;
- `GET /api/0/buckets/` for AFK bucket discovery;
- `GET /api/0/buckets/{bucket_id}/events?start=...&end=...` for bounded event
  reads.

The adapter does not rely on the query language, Swagger explorer, server
filesystem, or process ownership. `aw-server-rust` is the primary live smoke
target. Python `aw-server` runs the same contract suite. A `401` response causes
onboarding/settings to request an API key; the key is protected with current
user DPAPI and never enters logs or diagnostic exports.

## Data flow

1. On startup, validate configuration and start the selected activity source:
   start built-in input polling, or connect to ActivityWatch.
2. With ActivityWatch, probe server capabilities and discover compatible AFK
   buckets for the current host/device.
3. Register Windows session and power notifications.
4. Reconcile and record the current Windows/tracking state in the local ledger.
5. On a periodic tick or source change, read activity intervals from the
   source selected for each instant, and local events.
6. Normalize, clamp, split at policy boundaries, and de-overlap intervals.
7. Apply the accounting matrix and append-only manual adjustments.
8. Recompute affected daily summaries and cross-day credit in date order.
9. Calculate remaining time, projection, indicator state, and source quality.
10. Publish one immutable summary to all views and notification evaluation.
11. Persist configuration, WorkTime events/adjustments/receipts, and bounded
    diagnostics.

## UI proposal

- **Tray icon**: coloured circle plus a distinct degraded glyph.
- **Tooltip**: textual state and main value for accessibility.
- **Left click**: compact flyout with state, worked/target, projected departure
  or overtime, applied/available credit, source freshness, pending reviews, and
  a short labelled breakdown.
- **Main/detail view**: current-day interval breakdown plus Monday–Friday
  summaries and editing for the current week; no advanced charts in v1.
- **Right click**: open, refresh, pause/resume, finish day, add time, settings,
  diagnostics, quit.
- **Settings window**: General, Schedule, Accounting, Activity source,
  Notifications, About. Activity source chooses built-in or ActivityWatch,
  sets the built-in idle threshold, and holds the ActivityWatch connection
  settings.
- **Onboarding**: language, initial schedule settings, start-with-Windows
  consent, and confirmation. The built-in source needs no step. When a
  compatible ActivityWatch server answers at the default address, onboarding
  offers it, with its optional API key; otherwise it says nothing about
  ActivityWatch.
- **Notifications**: independent “approaching completion” and “approaching
  latest departure” Windows notifications with persisted once-per-day
  deduplication; merge both when eligible in the same minute.

Windows does not guarantee a persistent textual value adjacent to a standard
tray icon. The flyout/tooltip approach is therefore the proposed interpretation
of “pastille verte — 17h30”.

All user-visible WinUI strings live in French and English `.resw` resources.
Colour states use text and iconography in addition to hue.

## Project structure

```text
src/
  WorkTime.App/                 # WinUI 3 entry point, views, view-models
  WorkTime.Domain/              # pure accounting and forecasting
  WorkTime.ActivityWatch/       # read-only REST adapter
  WorkTime.Persistence/         # SQLite, JSON settings, DPAPI secret store
  WorkTime.Windows/             # session, power, startup, tray, notifications
tests/
  WorkTime.Domain.Tests/
  WorkTime.ActivityWatch.Tests/
  WorkTime.Persistence.Tests/
  WorkTime.Windows.Tests/
  WorkTime.App.Tests/
packaging/
  WorkTime.Package/             # MSIX manifest and release assets
```

## Testing strategy

- table-driven unit tests for interval arithmetic and every threshold boundary;
- property tests for invariants such as no double counting and totals never
  exceeding covered wall-clock duration;
- frozen-clock tests across lunch, midnight, DST, and setting changes;
- multi-day scenario tests for credit earning, consumption, expiry, and
  retroactive corrections;
- notification threshold/deduplication tests across restarts;
- built-in activity source tests on an injected clock and a fake input-tick
  reader: threshold crossing, backdated idle start, suspend in the middle of
  an active period, lock, restart and crash recovery, and tick wraparound;
- source selection tests proving each instant is read from one source only;
- ActivityWatch contract fixtures for bucket discovery and event shapes;
- adapter tests against both disposable `aw-server-rust` and Python `aw-server`
  smoke instances;
- Windows tests for session-state mapping;
- Windows tests for suspend/resume exclusion and startup registration;
- packaged-app smoke checklist on a clean supported Windows VM;
- localization resource completeness checks for French and English;
- diagnostics tests proving secrets and ActivityWatch window/app data are
  absent from exports;
- performance smoke test enforcing under 0.5% average idle CPU, 150 MiB working
  set, and a usable tray state within 3 seconds on the release VM.

## Packaging and updates

Revised on 2026-10-11 (#108, D-016): v1 ships through the Microsoft Store.

Alpha CI keeps publishing a self-contained, unsigned Windows x64 ZIP (and an
unsigned validation MSIX) to the `alpha-latest` GitHub release. Public v1 is an
x64 MSIX submitted to the Microsoft Store. Windows 10 and ARM64 packages are
outside v1.

The Store signs the package after certification, so the project holds no
code-signing certificate and no signing secret. Submission runs from a
`msstore` (Microsoft Store Developer CLI) job in `.github/workflows/release.yml`
on `v*` tags, behind a protected GitHub `release` environment with required
approval; its Partner Center credentials are environment secrets. The owner
first reserves the app name in Partner Center, which assigns the package
`Identity Name` and `Publisher`; the manifest in `packaging/WorkTime.Package/`
still uses the placeholders `Thanaen.WorkTime` and `CN=Thanaen` until then. The
Store listing also needs a privacy policy URL (it must disclose the anonymous daily usage
signal, D-019).

Updates:

- The Store installs updates in the background according to the user's Store
  settings.
- The Store-installed build also checks at most once per day when enabled:
  `StoreContext.GetDefault()` then `GetAppAndOptionalStorePackageUpdatesAsync()`.
  When updates exist, WorkTime offers them; only an explicit user action calls
  `RequestDownloadAndInstallStorePackageUpdatesAsync`, which lets Windows ask
  for the user's permission before downloading. WorkTime never calls the
  `TrySilentDownload*` methods. In a WinUI 3 desktop app, `StoreContext` must be
  given an owner window with `WinRT.Interop.InitializeWithWindow.Initialize`
  before the first call, discovery included, and the calls run on the UI
  thread; otherwise discovery fails with `ERROR_INVALID_WINDOW_HANDLE`. Sources:
  [Download and install package updates from the Store](https://learn.microsoft.com/en-us/windows/uwp/packaging/self-install-package-updates),
  [Display WinRT UI objects that depend on CoreWindow](https://learn.microsoft.com/en-us/windows/apps/develop/ui/display-ui-objects),
  [GetAppAndOptionalStorePackageUpdatesAsync](https://learn.microsoft.com/en-us/uwp/api/windows.services.store.storecontext.getappandoptionalstorepackageupdatesasync).
- The portable build has no package identity and no in-app update check.
  WorkTime no longer polls GitHub Releases. Known limitation: testers update
  the portable ZIP by hand, by downloading a newer one from `alpha-latest`.

Portable and packaged builds share all domain and persistence code. Any reduced
startup or notification behavior in portable builds is documented in the alpha
release notes and covered by an explicit smoke checklist.

Open item: the packaged app's writes under `%LocalAppData%` are virtualized.
On Windows 10 1903 and later, new files and folders are redirected to a
per-package private location, while a file that already exists in the real
`AppData` location is opened there
([How packaged desktop apps run on Windows](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)).
The data of a former portable alpha in `%LocalAppData%\WorkTime` must still be
found and kept consistent (new files such as SQLite journals or logs would land
in the private copy). Options: move the data to the package's own folder with
an import on first launch, or accept the merged view after testing it on
Windows. Disabling virtualization in the manifest
(`desktop6:FileSystemWriteVirtualization`) needs the restricted capability
`unvirtualizedResources`, which Microsoft documents as intended only for
certain games, so it is unlikely to pass Store certification
([reference](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop6-filesystemwritevirtualization)).

## Diagnostics and retention

- no telemetry beyond the anonymous daily usage signal (D-019), and no
  automatic crash upload;
- structured rolling local logs capped at 20 MiB and 14 days, whichever comes
  first;
- no ActivityWatch event payloads beyond non-sensitive identifiers and counts
  in logs;
- explicit ZIP diagnostic export with settings secrets removed;
- WorkTime accounting history is not automatically expired in v1.

## Delivery slices

1. Pure calculation core with fixture timelines and a CLI/test harness.
2. Read-only ActivityWatch adapter and diagnostic source view.
3. SQLite event ledger and Windows session-state watcher.
4. Tray icon, tooltip, and compact flyout.
5. Pause/end-day controls, review, and manual adjustments.
6. Cross-day credit ledger and multi-day recalculation.
7. Settings, notifications, and immediate recalculation.
8. Onboarding, French/English resources, update check, and diagnostics.
9. Packaging, startup registration, accessibility, and release hardening.
10. Built-in activity source as the default, source selection in settings,
    and onboarding without the ActivityWatch prerequisite (D-021).

Supporting artifacts:

- [Research and decisions](research.md)
- [Data model](data-model.md)
- [Built-in activity source contract](contracts/built-in-activity-source.md)
- [ActivityWatch REST contract](contracts/activitywatch-rest.md)
- [Quickstart and validation scenarios](quickstart.md)
