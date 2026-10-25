# Product Decisions

This file records ratified choices and the remaining clarification queue for the
first complete WorkTime specification.

## Ratified on 2026-07-27

### D-001 — Accounting posture

WorkTime is a personal, local indicator rather than a canonical timesheet.
Transparency and easy correction matter more than audit-grade immutability.

### D-002 — Compact tray experience

Use a coloured tray icon, textual tooltip, and a left-click compact flyout. The
departure time is shown in the flyout and the tooltip, all day since D-015 was
revised, because Windows does not reliably support permanent text beside a
standard tray icon.

### D-003 — Default accounting matrix

- unlocked + ActivityWatch `not-afk`: credited as active work;
- unlocked + ActivityWatch `afk`: excluded;
- locked inside the usual lunch window: excluded and labelled `lunch-lock`;
- locked outside the usual lunch window: credited and labelled `locked-work`;
- locked when the workday ends: excluded and labelled `overnight-lock`;
- unlocked and active inside the usual lunch window: credited as active work.

An outside-lunch lock is credited provisionally while its workday is live. Long
locks are flagged for confirmation, and an explicit “finish day” action prevents
unbounded accrual after departure.

After 90 continuous locked minutes outside lunch, the interval remains
provisionally credited and continues to affect the live colour/projection, but
the day is marked for review. After the unlock, WorkTime asks whether the
interval was work or the end of the day.

A lock that is not lifted before the end of its workday marks the end of that
day. The coverage from the lock to the end of the workday is excluded as
`overnight-lock`, and the next workday excludes the same lock until the unlock.
Neither day creates a review, and nothing is credited until the user declares
the interval as work from history. The trade-off: a lock for a meeting followed
by a shutdown from the lock screen, without unlocking, is excluded entirely and
must be declared manually.

Unresolved long-lock coverage contributes to the live result but does not earn
transferable weekly credit. Unlocking ends the interval; it does not confirm
that the interval was work. The review marker stays visible until the coverage
is resolved. Short locks below the review threshold retain their existing
credit behavior. Corrections recalculate the available credit for later days.
Reviews and corrections are scoped to each workday; declaring an overnight lock
as work therefore requires a correction on each affected day.

Revised on 2026-10-11 (D-021): the first two rows no longer name
ActivityWatch. They read “unlocked + active: credited as active work” and
“unlocked + idle: excluded”. Active and idle come from the selected activity
source: the built-in source's `active` and `idle`, or ActivityWatch's
`not-afk` and `afk`. The other rows are unchanged.

Revised on 2026-10-12 (#136): a lock that crosses one boundary of the usual
lunch window, and stays outside it for less than the long-lock review
threshold, is excluded as `lunch-lock` in full. It is a lunch taken a little
early or late, so it creates no review. Example: locked 11:50, unlocked 13:29,
lunch 12:00–14:00, threshold 45 min. The 10 minutes before 12:00 are lunch,
not provisional work. A lock that spans the whole lunch window, or stays
outside it for the threshold or longer, keeps the rules above.

### D-004 — Overtime definition

Overtime is credited work beyond the applicable target, not wall-clock time
elapsed since the target was first reached. It advances only while time is being
credited.

### D-005 — Manual controls and corrections

V1 includes:

- pause and resume tracking;
- an explicit “finish day” action;
- reclassifying or correcting a suspicious interval;
- adding missing work time manually.

Corrections MUST remain visible in the daily breakdown and MUST NOT rewrite
ActivityWatch-owned data.

“Finish day” stops automatic accrual until an explicit resume or the next
Monday–Friday workday. Saturday and Sunday remain inactive by default.

### D-006 — Simple weekly schedule

V1 uses one configuration for Monday through Friday. Per-day targets, general
days-off calendars, and workdays spanning midnight are out of scope.

### D-007 — Cross-day overtime credit

V1 supports carrying surplus credited time to later workdays so, for example,
Monday overtime can reduce Tuesday's remaining target.

- Only positive credit carries; under-time never creates debt.
- Credit is consumed automatically on later workdays in the same week.
- Unused credit expires after Friday.
- Credit cannot reduce the effective target below a configurable daily floor,
  default 6 hours.
- Credit not applicable because of the floor remains available for another
  workday until weekly expiry.

Example: with a 7-hour nominal target, +30 minutes earned Monday makes Tuesday's
effective target 6h30. At 6h30 worked Tuesday, the indicator becomes blue and
shows `6h30 worked · 30 min credit used`. Tuesday's new overtime starts at zero
and grows only after 6h30.

### D-008 — Notifications

V1 supports configurable Windows notifications:

- a chosen number of credited minutes before completing the applicable target;
- a chosen number of wall-clock minutes before the preferred latest departure.

The two triggers are enabled by default with a 15-minute lead and can be
configured independently.

- Each trigger fires at most once per workday, including across app restarts.
- The latest-departure warning fires only while the effective target remains
  incomplete and includes remaining work plus projected departure.
- The completion warning may still fire later even if the latest-departure
  warning already fired.
- If both triggers become eligible in the same minute, WorkTime merges them into
  one notification.
- A separate “day complete” notification is disabled by default.

### D-009 — Passive work and AFK

ActivityWatch `afk` time is excluded by default, including passive meetings or
media with no keyboard/mouse activity. V1 relies on an easy manual correction
for legitimate passive work. A local application allowlist may be considered
later; counting every unlocked minute is explicitly rejected.

Revised on 2026-10-11 (D-021): this applies to idle time from either activity
source. The built-in source uses the same signal and default threshold as
`aw-watcher-afk`, so passive meetings stay idle in the same way.

### D-010 — Suspend and hibernation

Windows suspend and hibernation time does not count automatically, even if the
session was locked beforehand. A suspend event stops accrual immediately. Resume
starts a new interval according to the current session and tracking state.

An explicit manual interval work adjustment or reclassification can credit selected
suspended coverage when the user worked away from the computer. A general long-lock
confirmation still preserves every suspended interval that it spans.

Revised on 2026-10-12 (#136): history no longer asks the user to settle
suspended, powered-off, or night-lock time that lies before the day's first
work, or after the last work of a finished day. Only pauses and other such
time may separate it from the edge of the day, and the day must contain work
(active, manual, or locked time awaiting review). That time stays excluded
and leaves the day's strip, like idle time before arrival. Such time between
two pieces of the day, after the last work of the current day, or on a day
without work, is still offered for settling. Work done away from a sleeping
computer before arrival can still be declared with a manual work interval.

Revised on 2026-10-25 (#11): a manual credit of suspended time inside a long
lock survives the lock's confirmation, whether the lock is confirmed from its
review or settled by a range counted as work, and can be added after it, so
counting never lowers the day.

### D-011 — Incomplete source history and autostart

When WorkTime starts after work began:

- ActivityWatch intervals known to be active may be reconstructed and credited;
- time whose session/activity state cannot be established is labelled unknown
  and never credited automatically;
- the summary exposes the uncertainty instead of guessing.

To minimize gaps, first-run onboarding offers “Start with Windows”, selected by
default but requiring explicit user confirmation before registration.

Revised on 2026-10-11 (D-021): activity before WorkTime starts can be
reconstructed only when ActivityWatch is the selected source and was running
at that time. The built-in source observes activity only while WorkTime runs,
so with the default source the time before launch, or between a crash and the
next start, is unknown. Lock and power history from the Windows event logs is
still reconstructed at startup, and unknown time is still shown and never
credited automatically. “Start with Windows” remains the main mitigation.

Revised on 2026-10-19 (#301): the flyout's unrecorded-time notice no longer
counts unknown time before the day's first work. That time runs from midnight
and is most often the night before arrival, not work to declare. Unknown time
after the first work, or on a day without work, is still reported. Work done
before WorkTime started can still be declared with a manual work interval.

### D-012 — History scope

V1 shows a detailed current day plus summary and editing for the five workdays
of the current week. WorkTime-owned local history is retained without automatic
expiry. Advanced charts and long-range analytics are out of scope.

## Ratified technical direction

### D-013 — Technical baseline

Use the latest stable LTS .NET and C# at implementation start, currently .NET 10
LTS and C# 14, with WinUI 3 / Windows App SDK. V1 supports Windows 11 x64 only.
Windows 10 and ARM64 are deferred.

WinUI 3 has no native `NotifyIcon`; use the stable `H.NotifyIcon.WinUI` package
behind a small tray abstraction unless an implementation spike finds a blocking
accessibility or lifecycle issue.

### D-014 — WorkTime persistence

Keep ActivityWatch strictly read-only. Store WorkTime-owned session transitions,
pause/end-day events, manual adjustments, cross-day credit, and notification
receipts in one local SQLite ledger. Store low-volume preferences in JSON.

This replaces the earlier WorkTime-owned ActivityWatch bucket proposal. The
extra local ledger is now justified by the correction and cross-day features
and removes write coupling to ActivityWatch's evolving API.

Revised on 2026-10-11 (D-021): the same ledger also stores the built-in
source's active/idle spans and the history of activity source selections.
ActivityWatch, when selected, stays strictly read-only.

### D-015 — Forecast semantics

Before and during the usual lunch window, the projection reserves the
still-future lunch time. After lunch, it uses actual credited time and assumes
remaining elapsed minutes accrue one-for-one.

Revised on 2026-10-23: the departure time is shown all day. Until the
configured lunch end (14:00 by default), the flyout adds that lunch is planned
until then. Before this revision, the time stayed hidden until lunch end and the
flyout gave a qualitative margin instead, which read as too complex. Active work
inside the window is still credited, so the shown time can only move earlier
once lunch is over.

### D-016 — Distribution and updates

WorkTime targets a public v1 while the source repository stays private during
development. Whether the source later becomes open source is a separate
decision.

- Alpha builds ship as portable x64 ZIP artifacts.
- V1 ships as a signed x64 MSIX through GitHub Releases.
- WorkTime checks GitHub Releases at most once per day and offers available
  updates; it never installs silently.
- Microsoft Store distribution is deferred.

Revised on 2026-10-11 (#108): the list above is superseded. The owner chose the
Microsoft Store for v1 after the distribution study. An individual developer in
France cannot use Azure Artifact Signing, a separate OV certificate costs money
every year, and a newly signed installer still meets SmartScreen warnings. The
Store is free for individual developers, re-signs the MSIX, and updates it.

- Alpha builds still ship as unsigned portable x64 ZIP artifacts on GitHub
  Releases, for testers and machines without the Store.
- V1 ships as an x64 MSIX through the Microsoft Store. The Store signs the
  package after certification; the project holds no code-signing certificate.
- The Store updates the app in the background according to the user's Store
  settings. In addition, the packaged build asks the Store for updates
  (`Windows.Services.Store.StoreContext`) at most once per day when enabled and
  offers an available update; installing it takes an explicit user action.
  WorkTime itself never installs silently.
- WorkTime no longer polls GitHub Releases. The portable build has no in-app
  update check; testers download a newer ZIP themselves.
- A separately signed installer (Velopack) is the documented fallback if the
  Store blocks the release, for example on managed machines or after a
  certification refusal.

Revised on 2026-10-19: the owner found replacing the exe by hand after each
download a friction. Builds outside the Store now ship as an unsigned Velopack
installer (`Thanaen.WorkTime-win-Setup.exe`, per-user, no administrator
rights), built from Linux by `scripts/release-direct.sh` and published to
`https://worktime.thanaen.dev/updates/` without CI. The Store stays the target
of the public v1.

- The installed build (channel `direct`) reads that feed at most once per day
  while **Check for updates** is on, and downloads a newer package at once.
- The update installs at the next start of WorkTime, without a prompt, or
  right away when the user chooses **Restart WorkTime** in the flyout. This
  replaces "never installs silently" for this build: the owner asked for the
  least friction.
- Uninstalling removes the start-with-Windows value and the toast registration;
  the data folder stays.

### D-017 — ActivityWatch implementations

`aw-server-rust` is a first-class and primary tested backend. The Python
`aw-server` remains supported through the shared read-only REST subset:
`GET /api/0/info`, `GET /api/0/buckets/`, and
`GET /api/0/buckets/{id}/events`.

The default endpoint is `http://127.0.0.1:5600` and remains configurable.
WorkTime capability-probes the server rather than assuming an implementation
from its process name. Optional `aw-server-rust` bearer authentication is
supported without storing the API key in plain-text settings.

If no compatible server/AFK bucket is available, onboarding explains the
prerequisite and links to ActivityWatch's official installation page. WorkTime
does not bundle, launch, stop, or reconfigure ActivityWatch.

Revised on 2026-10-11 (D-021): ActivityWatch is no longer a prerequisite. It
is an optional activity source. The REST subset, endpoint, and authentication
rules above apply only when the user selects it. When no compatible server is
found, onboarding continues with the built-in source and shows no installation
step or link. WorkTime still does not bundle, install, launch, stop, or
reconfigure ActivityWatch.

### D-018 — Localization

V1 ships French and English resources. The default follows the Windows display
language with an in-app override. User-visible strings do not live directly in
view code.

### D-019 — Privacy and diagnostics

V1 sends one anonymous daily usage signal (version, channel, first-of-month
flag), on by default, disclosed at onboarding and switchable off in Settings.
It sends no identifier, no time-tracking data and no crash report. Diagnostics
consist of bounded local logs with redaction and an explicit user-triggered
export. The export excludes ActivityWatch window titles and application
history.

Revised on 2026-10-12 (#158): the anonymous daily usage signal replaces "no
usage telemetry".

### D-020 — Packaging boundaries

Packaging differences MUST NOT leak into the calculation core. Portable alpha
builds may have reduced OS integration where Windows requires package identity;
the release checklist must state those differences. The signed MSIX is the
normative v1 experience for startup registration and notifications.

Revised on 2026-10-11 (#108): the normative v1 experience is the MSIX installed
from the Microsoft Store (Store-signed). The unsigned MSIX built by CI is for
validation only. The portable ZIP keeps the unpackaged workarounds (`Run` key,
COM notification registration).

### D-021 — Built-in idle detection as the default activity source

Ratified on 2026-10-11 by the owner, after the ActivityWatch study (option B).
WorkTime reads only one signal from ActivityWatch: whether the keyboard or
mouse was used recently. On Windows that signal comes from one system call
that WorkTime can make itself. Installing the whole ActivityWatch suite, window
tracking included, to read one boolean is rejected, and so is installing
ActivityWatch automatically.

- WorkTime has two activity sources behind `IActivitySourcePort`: the built-in
  source (the default) and ActivityWatch (optional).
- The built-in source calls `GetLastInputInfo` every 5 seconds. The Windows
  call is an adapter in `WorkTime.Windows`; the sampling and state rules sit
  behind it and are tested with the injected clock. After an idle threshold without keyboard
  or mouse input, 180 seconds by default and adjustable in settings, the state
  becomes `idle` from the last input. The next input makes it `active` again.
  These are the same rules as `aw-watcher-afk`, so totals match.
- Transitions are stored in the local SQLite ledger, next to the session
  transitions. WorkTime stores only `active`/`idle` timestamps, never an
  application name or window title.
- The user chooses one source in settings: “Built-in” by default, or
  “ActivityWatch” once a connection test finds a compatible server. Each
  instant of history is read from exactly one source. Sources are never
  combined or merged for the same period, and WorkTime never falls back from
  one source to the other on its own.
- Onboarding no longer blocks on ActivityWatch. It probes the default address
  and offers ActivityWatch only when a compatible server is detected.
- Settings files written before this decision keep ActivityWatch as their
  selected source, because those users completed onboarding with it. On the
  first start after the upgrade, WorkTime records that choice as an
  ActivityWatch selection effective at the Unix epoch, so their earlier
  history stays with ActivityWatch. New installations start with the built-in
  source from onboarding confirmation; time before it is unknown, and
  ActivityWatch is not read for it. No source applies before the first
  recorded selection.
- The built-in source credits an `active` period only through the last input
  it read. Time it did not observe, and a pending tail cut short by a
  shutdown, crash, or suspend, stay unknown and are never credited later.

The trade-off is history before launch (see D-011): only ActivityWatch can
supply activity from before WorkTime started. Users who need that keep
ActivityWatch as their source.

The built-in source is Windows-specific. That adds no new platform dependency,
because the session, power, tray, and packaging code is already Windows-only.

The sampling, persistence, and read rules are in the
[built-in activity source contract](../specs/001-core-workday-indicator/contracts/built-in-activity-source.md).

### D-022 — v1 is a Rust rewrite with a GPUI interface

Decided on 2026-10-12 by the owner. The v1 release is a rewrite of WorkTime in
Rust, with an interface built on GPUI and GPUI Kit (`gpui-component`). It
replaces three earlier plans: keeping the WinUI interface, shrinking the
Windows App SDK build, and moving the .NET build to .NET 11.

- The rewrite lives in a Cargo workspace under `crates/`. The .NET solution
  stays on `main` and keeps working until the Rust build reaches parity, then
  it is removed. Until then it only receives fixes that block daily use.
- The Rust build reads the same SQLite database and `settings.json` as the
  .NET build, at the same location, and continues the schema numbering. A user
  who switches builds keeps their history without a conversion step.
- Calculation rules are ported with differential tests: the .NET test suite
  exports its cases as JSON fixtures, and the Rust domain must produce the same
  results.
- The built-in idle source (D-021) is the only activity source in v1.
  ActivityWatch is not ported; history already recorded from it stays
  readable.
- A first GPUI prototype (tray icon and day flyout) must meet size, startup,
  memory and screen-reader targets before the other screens are built.

The phases and their order are tracked in GitHub issue #145.

## Later decisions

- whether and under which license to open-source the repository;
- Windows on ARM64;
- localization beyond French and English;
- optional calendar-aware projections;
- multiple ActivityWatch hosts/devices;
- public extension/plugin model;
