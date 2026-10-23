# Feature Specification: Core Workday Indicator

**Feature Branch**: `001-core-workday-indicator`  
**Created**: 2026-07-27  
**Status**: Ready
**Input**: Initial WorkTime product brief
**Revised**: 2026-10-11, built-in idle detection becomes the default activity
source and ActivityWatch becomes optional (D-021)

## Product intent

WorkTime helps a Windows user understand, without opening a time-tracking
dashboard, whether they can still complete their daily work target before their
preferred latest departure time.

It is an indicator and forecasting tool, not an employee-monitoring product,
payroll system, or canonical timesheet.

## User Scenarios & Testing

### User Story 1 — Understand today's trajectory (Priority: P1)

As a user, I can glance at the tray icon and know whether I am comfortably on
track, approaching my preferred latest departure, already projected to exceed
it, or finished for the day.

**Why this priority**: This is the product's core value and must work without
opening a full application window.

**Independent Test**: Feed a fixed configuration and deterministic day timeline
to the calculation core and verify the resulting state at representative
instants and exact threshold boundaries.

**Acceptance Scenarios**:

1. **Given** a projected departure earlier than or equal to the warning
   threshold, **when** the indicator refreshes, **then** it is green.
2. **Given** a projected departure later than the warning threshold but not
   later than the preferred maximum, **when** the indicator refreshes, **then**
   it is orange.
3. **Given** a projected departure later than the preferred maximum, **when**
   the indicator refreshes, **then** it is red.
4. **Given** credited work equal to or greater than today's effective target
   after applied carry, **when** the indicator refreshes, **then** blue
   overrides the forecast colours.
5. **Given** colour is unavailable or difficult to distinguish, **when** the
   user opens or hovers the indicator, **then** the same state is communicated
   in text.

### User Story 2 — See a useful forecast and overtime (Priority: P1)

As a user, after my usual lunch window I can see my projected departure time if
I keep accruing work, and after completing the target I can see overtime.

**Why this priority**: The colour says whether action is needed; the value says
how much.

**Independent Test**: Evaluate fixed timelines before, during, and after lunch,
then after target completion, and verify visibility and values.

**Acceptance Scenarios**:

1. **Given** the current time is before the configured forecast-display point,
   **when** the compact view appears, **then** no departure time is shown.
2. **Given** the display point has passed and the target is incomplete, **when**
   the compact view appears, **then** it shows the projected local departure
   time.
3. **Given** the daily target is complete, **when** the compact view appears,
   **then** it shows credited overtime instead of a projected departure.
4. **Given** source data is unavailable or stale, **when** the compact view
   appears, **then** it shows an explicit unavailable/degraded state rather than
   a misleading forecast.

### User Story 3 — Account for activity and locked sessions (Priority: P1)

As a user, active computer time and relevant locked-session time are classified
according to explicit rules so my daily total matches how I intend to account
for work.

**Why this priority**: A polished indicator is useless if the underlying total
cannot be trusted.

**Independent Test**: Provide overlapping activity-source, Windows
session-state, and lunch-window intervals and verify the credited, excluded, and
labelled totals.

Active and idle come from the selected activity source (FR-044, FR-045): the
built-in source's `active` and `idle`, or ActivityWatch's `not-afk` and `afk`.

**Default accounting matrix**:

- unlocked + active: credited as active work;
- unlocked + idle: excluded;
- locked inside the usual lunch window: excluded and labelled `lunch-lock`;
- locked outside the usual lunch window: credited and labelled `locked-work`;
- locked when the workday ends: excluded and labelled `overnight-lock`.

Unlocked active time inside the lunch window remains credited. Outside-lunch
locks remain provisionally credited and affect the live result while the
workday is live. After 90 continuous locked minutes, the day is marked for
review until the user confirms whether the interval was work or the end of the
day. A lock that is not lifted before the end of its workday marks the end of
that day: the rest of that day and the next workday until the unlock are
excluded as `overnight-lock` without a review. A lock for a meeting followed by
a shutdown from the lock screen is therefore excluded entirely and must be
declared as work from history.

Revised on 2026-10-12 (#136, D-003): a lock that crosses one lunch boundary
and stays outside lunch for less than the review threshold is excluded in full
as `lunch-lock`, without a review.

**Acceptance Scenarios**:

1. **Given** overlapping source events, **when** totals are computed, **then**
   no wall-clock interval is counted twice.
2. **Given** a lock crossing a lunch boundary, **when** totals are computed,
   **then** the interval is split and each segment receives the applicable rule.
3. **Given** a labelled locked-work interval, **when** the user opens the daily
   breakdown, **then** it remains distinguishable from active work.
4. **Given** WorkTime restarts, **when** it reads the selected activity source
   again, **then** persisted source history can reconstruct completed intervals
   without silently losing or duplicating time.
5. **Given** a lock exceeds the configured suspicious duration during a live
   workday, **when** the user next interacts with WorkTime, **then** the
   interval is clearly marked for confirmation rather than silently accepted.
6. **Given** a lock is still active when its workday ends, **when** that day or
   the next one is computed, **then** the locked coverage up to the unlock is
   excluded as `overnight-lock` without a review and can be declared as work
   from history.

### User Story 4 — Configure personal rules (Priority: P2)

As a user, I can configure my target and forecast thresholds without editing a
file.

**Independent Test**: Change each setting, restart the application, and verify
that the setting and recalculated indicator persist.

**Required settings**:

- daily work target, default 7 hours;
- usual lunch start and end, default 12:00–14:00;
- preferred latest departure, default 18:30;
- orange warning margin, default 15 minutes;
- launch at Windows sign-in;
- activity source, built-in by default, or ActivityWatch once detected;
- idle threshold of the built-in source, default 180 seconds;
- long-lock review threshold;
- cross-day credit policy;
- minimum effective daily target, default 6 hours;
- completion and latest-departure notification settings.

**Acceptance Scenarios**:

1. **Given** first launch, **when** settings load, **then** documented defaults
   are active.
2. **Given** a valid setting change, **when** it is saved, **then** the current
   day is recalculated immediately.
3. **Given** an invalid or contradictory value, **when** the user attempts to
   save, **then** WorkTime explains the problem and retains the last valid
   configuration.

### User Story 5 — Control and correct the day (Priority: P1)

As a user, I can pause automatic accounting, finish my day, correct a
misclassified interval, or add missing work without altering the activity
source's data.

**Independent Test**: Apply each control to a fixed timeline, restart WorkTime,
and verify that the adjustment persists, remains visible, and deterministically
changes the summary.

**Acceptance Scenarios**:

1. **Given** tracking is paused, **when** the activity source reports activity
   or the Windows session changes, **then** the elapsed interval is excluded and
   labelled `manual-pause`.
2. **Given** the user chooses “finish day”, **when** later activity occurs that
   day, **then** it is not automatically credited unless the user resumes.
3. **Given** the user does not resume after finishing a day, **when** the next
   Monday–Friday workday begins, **then** automatic accounting resumes; weekends
   remain inactive by default.
4. **Given** a suspicious locked interval, **when** the user reclassifies it,
   **then** the daily summary updates immediately and retains an explicit manual
   adjustment.
5. **Given** missing source time, **when** the user adds a valid manual work
   interval or duration, **then** it is credited once and labelled `manual`.
6. **Given** an adjustment overlaps existing credited time, **when** it is
   saved, **then** WorkTime prevents double counting.

### User Story 6 — Use surplus on another workday (Priority: P2)

As a user, credited overtime from an earlier workday can reduce the remaining
target on a later workday.

**Independent Test**: Complete a deterministic sequence of Monday–Friday
summaries and verify accrual, consumption, expiry, corrections, and the
effective target shown each day.

**Acceptance Scenarios**:

1. **Given** Monday ends with positive transferable overtime, **when** Tuesday
   begins, **then** the applicable credit and reduced remaining target are
   visible.
2. **Given** carried credit completes today's effective target, **when** the
   summary refreshes, **then** the indicator becomes blue and explains how much
   time was worked today versus supplied by credit.
3. **Given** an earlier day's correction changes transferable overtime,
   **when** WorkTime recalculates, **then** later balances are recomputed without
   creating or losing minutes.
4. **Given** a week boundary, **when** the next workweek starts, **then** credit
   remaining from Friday has expired.
5. **Given** enough credit to reduce today's work below the configured floor,
   **when** today's effective target is calculated, **then** only applicable
   credit is consumed and the remainder stays available until Friday expiry.

### User Story 7 — Receive useful approach warnings (Priority: P2)

As a user, I can receive a Windows notification shortly before completing my
effective target and/or shortly before my preferred latest departure.

**Independent Test**: Advance a frozen clock and credited-time source across
each configured threshold, including restarts and recalculations, and verify
that each eligible notification is delivered at most once.

**Acceptance Scenarios**:

1. **Given** the completion notification is enabled, **when** remaining credited
   work first falls to its configured lead time, **then** WorkTime notifies once
   for that workday.
2. **Given** the latest-departure notification is enabled and the target remains
   incomplete, **when** wall-clock time reaches its configured lead time,
   **then** WorkTime notifies once with the useful remaining/projection context.
3. **Given** either notification is disabled, **when** its threshold is crossed,
   **then** no notification is sent.
4. **Given** WorkTime restarts after a notification was delivered, **when** the
   same threshold is evaluated again, **then** it does not duplicate the
   notification.
5. **Given** the latest-departure warning was already delivered, **when** the
   completion threshold is crossed later, **then** the completion warning is
   still delivered once.
6. **Given** both triggers become eligible in the same minute, **when**
   notifications are evaluated, **then** WorkTime sends one combined
   notification.

### User Story 8 — Diagnose source problems (Priority: P2)

As a user, I can tell whether the selected activity source is working: for the
built-in source, whether it is recording; for ActivityWatch, whether it is
installed, reachable, compatible, and producing recent data.

**Independent Test**: Simulate a failing built-in input query, then for
ActivityWatch a missing server, empty buckets, stale data, malformed events,
and recovery.

**Acceptance Scenarios**:

1. **Given** the selected ActivityWatch is unreachable, **when** WorkTime refreshes, **then**
   it enters a distinct degraded state and exposes recovery guidance.
2. **Given** ActivityWatch becomes healthy again, **when** the next refresh
   succeeds, **then** WorkTime recalculates the day without requiring a restart.
3. **Given** multiple candidate ActivityWatch buckets, **when** automatic
   selection is ambiguous, **then** WorkTime exposes the selected source or asks
   the user to choose.
4. **Given** WorkTime starts after work began, **when** it reconstructs the day,
   **then** known active intervals from the selected source are credited and
   unknown gaps remain visibly uncredited. With the built-in source, activity
   before WorkTime started is unknown; with ActivityWatch, its recorded
   intervals are known.
5. **Given** Windows suspends or hibernates, **when** the day is calculated,
   **then** the sleeping interval is excluded regardless of prior lock state.
6. **Given** the built-in source cannot query input activity, **when** WorkTime
   refreshes, **then** it enters a distinct degraded state, never treats the
   missing coverage as idle or active, and recovers when the query succeeds
   again.

### User Story 9 — Review the current week (Priority: P2)

As a user, I can understand and correct today in detail and review the five
workdays of the current week without using ActivityWatch's dashboard.

**Independent Test**: Populate five deterministic daily summaries and
adjustments, navigate the week, edit an eligible day, restart, and verify that
the history and recalculated credit persist.

**Acceptance Scenarios**:

1. **Given** the current day has credited, excluded, manual, unknown, and
   provisional intervals, **when** its detail opens, **then** each category and
   total is visible.
2. **Given** the current Monday–Friday week has multiple summaries, **when** the
   week view opens, **then** it shows worked time, target, earned/used credit,
   and review state per day.
3. **Given** a current-week day is corrected, **when** the change is saved,
   **then** all later affected credit balances and summaries update.
4. **Given** WorkTime restarts, **when** history reopens, **then** locally stored
   records remain available without an automatic expiry policy.

### User Story 10 — Set up and maintain WorkTime safely (Priority: P2)

As a Windows user, I can install WorkTime and have it count activity without
installing anything else, optionally connect it to either supported local
ActivityWatch server, choose my language and startup preference, diagnose
problems, and opt into an offered update without exposing activity data.

**Independent Test**: Run first-launch and update scenarios on a clean Windows
11 x64 VM without ActivityWatch, then against `aw-server-rust`, Python
`aw-server`, optional bearer authentication, French Windows, and English
Windows.

**Acceptance Scenarios**:

1. **Given** no compatible ActivityWatch server answers at the default address,
   **when** onboarding runs, **then** it completes with the built-in source,
   shows no ActivityWatch step, prerequisite, or installation link, and
   WorkTime is healthy after confirmation.
2. **Given** `aw-server-rust` or Python `aw-server` exposes the compatible REST
   subset and an AFK bucket at the default address, **when** onboarding runs,
   **then** it offers ActivityWatch as an alternative to the built-in source,
   which stays selected unless the user chooses ActivityWatch.
3. **Given** the user chooses ActivityWatch and `aw-server-rust` requires
   bearer authentication, **when** the user provides a valid API key, **then**
   WorkTime connects without storing the key in plain-text settings.
4. **Given** first launch, **when** the user confirms onboarding, **then** the
   selected start-with-Windows choice is applied.
5. **Given** Windows uses French or English, **when** WorkTime first starts,
   **then** it selects that supported language and permits an override.
6. **Given** the Store-installed build, a newer Store package, and the daily
   check is enabled, **when** WorkTime checks for updates, **then** it offers
   the update without installing it silently.
7. **Given** a support request, **when** the user exports diagnostics, **then**
   the archive contains bounded redacted WorkTime logs and no window titles or
   application history.

## Functional Requirements

- **FR-001**: WorkTime MUST run primarily as a Windows system-tray application.
- **FR-002**: WorkTime MUST calculate credited work for the current local
  workday from the selected activity source (FR-045) and WorkTime-recorded
  Windows session state.
- **FR-003**: WorkTime MUST refresh its result often enough that displayed
  minute values are no more than one minute behind healthy source data.
- **FR-004**: WorkTime MUST expose green, orange, red, blue, and degraded states.
- **FR-005**: Blue MUST take precedence once today's effective target is
  reached.
- **FR-006**: The orange boundary MUST be configurable and default to 15 minutes
  before the preferred latest departure.
- **FR-007**: WorkTime MUST show projected departure only after the configured
  display point and only while the target is incomplete.
- **FR-008**: WorkTime MUST preserve labelled totals for active work,
  locked-work, excluded lunch-lock, and other supported exclusion reasons.
- **FR-009**: WorkTime MUST detect Windows lock/unlock transitions while it is
  running and persist them in its local event ledger.
- **FR-010**: WorkTime MUST NOT mutate ActivityWatch-owned watcher buckets.
- **FR-011**: WorkTime MUST persist user configuration locally.
- **FR-012**: WorkTime MUST automatically recover from temporary unavailability
  of the selected activity source.
- **FR-013**: WorkTime MUST expose today's credited and excluded breakdown.
- **FR-014**: WorkTime MUST allow the user to enable or disable start with
  Windows after onboarding and MUST show the effective registration state.
- **FR-015**: WorkTime MUST use the machine's local timezone for display and
  store timestamps unambiguously.
- **FR-016**: WorkTime MUST display state by more than colour alone.
- **FR-017**: WorkTime MUST keep all processing local in v1.
- **FR-018**: V1 MUST use a single Monday–Friday schedule configuration and MUST
  NOT treat intervals crossing midnight as one workday.
- **FR-019**: WorkTime MUST support pause, resume, finish-day, interval
  correction/reclassification, and manual work entry.
- **FR-020**: Manual changes MUST be represented as WorkTime-owned adjustments
  and MUST NOT rewrite activity source records, whether built-in activity spans
  or ActivityWatch events.
- **FR-021**: WorkTime MUST flag long locked intervals for review after 90
  continuous minutes by default while continuing to include them provisionally
  in the live result. A lock still active when its workday ends MUST end that
  day: its remaining coverage, and the next workday's coverage until the
  unlock, MUST be excluded as `overnight-lock` without a review.
  Revised on 2026-10-12 (#136): a lock that crosses exactly one boundary of
  the usual lunch window and stays outside it for less than the review
  threshold MUST be excluded in full as `lunch-lock` without a review.
- **FR-022**: WorkTime MUST calculate and persist transferable positive overtime
  within a Monday–Friday week, MUST NOT carry negative debt by default, and
  MUST expire unused credit after Friday. Applied credit MUST NOT reduce the
  effective target below a configurable floor that defaults to 6 hours.
  Provisional coverage awaiting long-lock review MUST NOT earn transferable
  credit until confirmed, even after the lock has ended. Its contribution to
  the live daily result remains provisional under FR-021.
  Weekly debt is opt-in (off by default, effective only with weekly credit):
  a finished past workday with counted work under its effective target adds
  the shortfall to the week's debt, up to an optional configured maximum
  beyond which the excess is dropped. Workdays with no counted work and the
  running workday add none. Later surplus repays debt before it can be earned
  as transferable credit; work awaiting long-lock review counts toward the
  target but repays nothing until confirmed. Debt never raises a target, sits
  beside already-earned credit rather than consuming it, and ends with the
  week like credit. When available credit minus debt is negative, the tray,
  flyout, and history show that debt instead of available credit.
- **FR-023**: WorkTime MUST recalculate later credit balances when an earlier
  workday is corrected.
- **FR-024**: WorkTime MUST expose gross credited work, applied carry credit,
  effective remaining time, newly earned overtime, and remaining balance
  without conflating them.
- **FR-025**: WorkTime MUST support independently configurable completion and
  latest-departure notifications, both enabled by default with a 15-minute lead.
- **FR-026**: WorkTime MUST persist notification receipts so a restart cannot
  duplicate a notification for the same workday and trigger.
- **FR-027**: WorkTime MUST merge both warnings when they become eligible in the
  same minute, MUST allow a later completion warning after an earlier
  latest-departure warning, and MUST keep a separate day-complete notification
  disabled by default.
- **FR-028**: WorkTime MUST exclude idle time from the selected activity source
  by default (built-in `idle` or ActivityWatch `afk`) and MUST allow it to be
  corrected manually when it represents legitimate passive work.
- **FR-029**: WorkTime MUST exclude Windows suspend and hibernation intervals
  from automatic accounting regardless of the preceding session state. An explicit
  manual interval work adjustment or reclassification MAY credit selected suspended
  coverage. A general long-lock confirmation MUST preserve suspended coverage.
  Revised on 2026-10-12 (#136, D-010): history MUST NOT ask the user to settle
  suspended, powered-off, or `overnight-lock` time separated from the start of
  the day, or from the end of a finished day, only by such time and pauses,
  when the day contains work. It stays excluded. The same time between two
  pieces of the day, after the last work of the current day, or on a day
  without work MUST still be offered for settling.
- **FR-030**: When source history is incomplete, WorkTime MUST credit only known
  intervals, label unknown gaps, and surface their effect on confidence.
- **FR-031**: First-run onboarding MUST offer start-at-Windows-sign-in selected
  by default and MUST require explicit confirmation before enabling it.
- **FR-032**: WorkTime MUST expose detailed current-day accounting and a
  Monday–Friday current-week summary with editing and credit recalculation.
- **FR-033**: WorkTime-owned history MUST have no automatic expiry in v1.
- **FR-034**: V1 MUST support Windows 11 x64 and use the latest stable LTS .NET
  and C# at implementation start with WinUI 3 / Windows App SDK.
- **FR-035**: When ActivityWatch is the selected source, WorkTime MUST support
  `aw-server-rust` as a primary tested backend and Python `aw-server` through
  their shared read-only `/api/0` REST subset.
- **FR-036**: The ActivityWatch endpoint MUST default to
  `http://127.0.0.1:5600`, remain configurable, and support optional bearer
  authentication without plain-text secret storage.
- **FR-037**: ActivityWatch MUST be optional. Onboarding MUST complete with the
  built-in source when no compatible ActivityWatch server is detected, MUST
  offer ActivityWatch only when one is detected at the default address, and
  MUST NOT block on, link to, install, launch, stop, or reconfigure
  ActivityWatch. Incompatible servers and missing AFK buckets are diagnosed
  only when the user selects ActivityWatch.
- **FR-038**: V1 MUST ship French and English resources, select from the Windows
  display language by default, and allow an in-app override.
- **FR-039**: V1 MUST send at most one anonymous usage signal per day, holding
  only the app version, the install channel and a first-of-month flag. The
  signal MUST be on by default, disclosed at onboarding and switchable off in
  Settings, and turning it off MUST stop every send. WorkTime MUST send no
  identifier and no time-tracking data, MUST never send from debug builds, demo
  launches or launches with `--data-dir`, and MUST perform no automatic crash
  upload.
- **FR-040**: WorkTime MUST retain bounded redacted local logs and provide an
  explicit diagnostic export that excludes ActivityWatch application and window
  history.
- **FR-041**: Alpha builds MUST be publishable as unsigned portable x64 ZIP
  artifacts on GitHub Releases; public v1 MUST be publishable as an x64 MSIX
  through the Microsoft Store, which signs it.
- **FR-042**: The Store-installed build MUST ask the Microsoft Store for
  updates at most once per day when enabled, offer an available update, and
  install it only after an explicit user action. WorkTime MUST NOT install
  updates silently; the Store's own automatic updates follow the user's Store
  settings. The portable build has no in-app update check.
- **FR-043**: Windows 10 and ARM64 support are out of scope for v1.
- **FR-044**: WorkTime MUST provide a built-in activity source and use it by
  default. While WorkTime runs, the source MUST sample `GetLastInputInfo` every
  5 seconds. It MUST report `idle` from the last input once no input has
  occurred for the idle threshold, and `active` otherwise. It MUST persist
  these transitions in the local ledger and MUST NOT record application names
  or window titles. Time before WorkTime starts, or after the last persisted
  settled instant, MUST be unknown. An `active` span MUST be credited only
  through the last input a sample read: a pending tail interrupted by a
  shutdown, crash, or suspend MUST stay unknown, and elapsed wall time or a
  later threshold MUST NOT settle it. A gap in observation MUST NOT be closed
  by the next span.
- **FR-045**: The user MUST choose exactly one activity source in settings:
  built-in, or ActivityWatch once a connection test finds a compatible server.
  Each instant MUST be read from the source selected at that instant. WorkTime
  MUST NOT combine, merge, or automatically switch sources for the same period.
  Before the first recorded selection, activity MUST be unknown and WorkTime
  MUST NOT read any source. An installation upgraded from before D-021 MUST
  record ActivityWatch for all earlier history; a new installation MUST record
  its first selection at onboarding confirmation.
- **FR-046**: The built-in idle threshold MUST default to 180 seconds and MUST
  be configurable between 30 seconds and 30 minutes.

## Forecasting model

The ratified model is:

1. `effective target = max(configured floor, nominal daily target - carry
   applied to this workday)`.
2. `remaining = max(0, effective target - credited work so far)`.
3. Project from `now` assuming future elapsed minutes accrue one-for-one.
4. Add any still-future portion of the configured lunch window as expected
   non-working time.
5. Hide the numeric departure forecast until the configured lunch end, but
   continue using the internal forecast for the tray colour.
6. Compare the projected departure with:
   - green: `projection <= latest departure - warning margin`;
   - orange: above that threshold and `projection <= latest departure`;
   - red: `projection > latest departure`;
   - blue: target already complete.

Lunch is predictive before it occurs: active time during the window is credited,
while locked time is excluded. Credit is positive-only within the current
Monday–Friday week and cannot reduce the effective target below the configured
floor, default 6 hours.

## Key Entities

- **Workday Configuration**: target duration, lunch window, preferred maximum,
  warning margin, display policy, and accounting rules.
- **Activity Source Selection**: which activity source, built-in or
  ActivityWatch, applies from a given instant.
- **Activity Span**: a built-in source observation, `active` or `idle`, with
  its start and the latest instant at which its state is settled.
- **Source Interval**: a normalized interval from the selected activity source
  or Windows session state with source, start, end, and quality.
- **Accounted Interval**: a non-overlapping interval classified as credited,
  excluded, or uncertain, with a human-readable label.
- **Manual Adjustment**: an append-only correction, classification, pause,
  finish-day marker, or added work interval owned by WorkTime.
- **Credit Ledger Entry**: overtime earned, applied, expired, or recomputed for
  a workday, with provenance back to daily summaries and adjustments.
- **Notification Receipt**: a persisted record of a trigger delivered for a
  workday, used for deduplication.
- **Daily Summary**: credited totals by label, excluded totals, remaining time,
  applied and available credit, projection, indicator state, source freshness,
  pending reviews, and warnings.

## Edge Cases

- exact colour and target boundaries;
- app starts after work began or while Windows is already locked;
- lock begins before lunch and ends after it;
- computer sleeps, hibernates, reboots, or is left locked overnight;
- WorkTime starts after the first activity event of the day;
- a passive meeting is reported as AFK and later manually corrected;
- ActivityWatch starts late, stops temporarily, changes hostname, or has
  multiple AFK buckets;
- WorkTime crashes or is closed while the built-in source is recording;
- the user switches activity source in the middle of a workday;
- the 32-bit input tick count wraps after about 49.7 days of uptime;
- no activity yet today;
- user changes target or lunch settings mid-day;
- a past correction changes credit already consumed on later days;
- carry credit is greater than today's nominal target;
- both notification thresholds occur in the same minute;
- WorkTime starts after a notification threshold has already passed;
- daylight-saving transition or timezone change during a workday;
- target or maximum departure falls after midnight;
- device clock moves backward or forward;
- events overlap, contain gaps, arrive late, or extend into the future;
- multiple Windows user sessions or Remote Desktop sessions.

## Success Criteria

- **SC-001**: For a reviewed suite of representative timelines, credited work,
  projection, and colour exactly match the ratified rules.
- **SC-002**: A user can understand the current state and projected departure
  within two seconds of opening the compact tray view.
- **SC-003**: Healthy displays are no more than one minute behind the latest
  available source data.
- **SC-004**: Temporary activity source outages never crash WorkTime or replace
  the last healthy value with an apparently valid but incorrect value.
- **SC-005**: No activity or accounting data leaves the machine. The only
  optional remote request is a daily update query to the Microsoft Store from
  the Store-installed build.
- **SC-006**: On the release VM, healthy idle operation averages below 0.5% CPU
  over 15 minutes and uses no more than 150 MiB working set.
- **SC-007**: The packaged app reaches a usable tray state within 3 seconds of
  sign-in on the release VM with the built-in source, and with ActivityWatch
  already available when it is the selected source.
- **SC-008**: Contract tests pass against supported fixtures and live smoke
  instances of both `aw-server-rust` and Python `aw-server`. The built-in
  source passes injected-clock tests for the idle threshold, suspend during
  an active period, lock, and restart.
- **SC-009**: French and English UI resource validation reports no missing or
  hard-coded user-visible strings in release-critical views.

## Explicit non-goals for v1

- payroll, billing, or legally authoritative time sheets;
- employee monitoring or remote reporting;
- cloud sync, accounts, or team dashboards;
- macOS or Linux support;
- Windows 10 and Windows on ARM64;
- automatic interpretation of application names or window titles;
- calendar integration;
- mobile companion applications.

## Assumptions

- WorkTime works without ActivityWatch. Users who already run it locally may
  select it instead of the built-in source, for example to recover activity
  from before WorkTime starts.
- `aw-server-rust` is the primary ActivityWatch compatibility target; Python
  `aw-server` remains supported through the explicitly documented REST subset.
- A single Windows user and, when ActivityWatch is selected, one primary
  ActivityWatch host are sufficient for v1.
- WorkTime owns a local ledger for session state, built-in activity spans,
  activity source selections, manual adjustments, cross-day credit, and
  notification receipts; ActivityWatch remains read-only.
- Only active/idle data is needed for the default accounting model; window
  titles and application names are not read in v1.
- The compact view is accessed by clicking or hovering the tray icon; Windows
  does not provide a reliable persistent text label beside a normal tray icon.
