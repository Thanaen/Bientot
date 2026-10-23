# Data Model: Core Workday Indicator

**Status**: Ready  
**Storage**: Local SQLite plus JSON preferences

## Design rules

- Activity comes from exactly one selected source per instant: built-in
  (default) or ActivityWatch (D-021).
- Built-in activity spans are WorkTime-owned. ActivityWatch data is external,
  read-only, and never copied wholesale.
- UTC instants are authoritative; local dates and timezone IDs are explicit.
- Source events and manual adjustments are append-only where practical.
- Daily summaries and credit entries are derived and recomputable.
- No interval may contribute credited time more than once.

## Persistent entities

### SessionEvent

Records a WorkTime-observed Windows or tracking transition.

- `Id`: monotonic integer
- `OccurredAtUtc`: instant
- `TimezoneId`: Windows timezone at capture
- `Kind`: `locked`, `unlocked`, `suspend`, `resume`, `tracking-paused`,
  `tracking-resumed`, `day-finished`
- `Source`: Windows session, Windows power, or user
- `CreatedAtUtc`: audit instant

Transitions are ordered by `OccurredAtUtc`, then `Id`. Duplicate OS callbacks
with the same effective state are ignored idempotently.

### ActivitySpan

Records what the built-in activity source observed (see the
[built-in activity source contract](contracts/built-in-activity-source.md)).

- `Id`: monotonic integer
- `State`: `active` or `idle`
- `StartUtc`: instant the state began; an `idle` span starts at the last input
- `SettledThroughUtc`: latest instant at which the state is settled. For an
  `active` span, this is the last input a sample read, never the instant of a
  later sample, shutdown, suspend, or switch. For an `idle` span, it is the
  latest sample that read no new input.
- `CreatedAtUtc`: audit instant

Spans are ordered by `StartUtc`, then `Id`, and do not overlap. Only a
transition seen within uninterrupted observation closes the previous span, by
setting its `SettledThroughUtc` to the new span's `StartUtc`. A span appended
at start, at resume, or after missed samples leaves the previous span
unchanged. Time between one span's `SettledThroughUtc` and the next span's
`StartUtc` is unknown, as is everything before the first span. An `active`
span is credited only through `SettledThroughUtc`: a pending tail interrupted
by a shutdown, a crash, or a suspend stays unknown, and no later threshold or
elapsed wall time turns it into active time. Spans are written only while the
built-in source is selected and are never rewritten by corrections.

### ActivitySourceSelection

Records which activity source applies from a given instant.

- `Id`: monotonic integer
- `EffectiveFromUtc`: instant the selection takes effect
- `Source`: `built-in` or `activitywatch`
- `CreatedAtUtc`: audit instant

For any instant, the selection with the latest `EffectiveFromUtc` at or before
it decides the source; the source not selected is never read for that
instant. Before the first selection, no source applies: activity is unknown
and neither source is read. Every first row is written explicitly:

- A legacy installation is one whose saved settings file has no activity
  source value, because it was written before D-021 by an onboarding completed
  with ActivityWatch. On its first start after the upgrade, before any read,
  WorkTime appends an `activitywatch` selection effective at the Unix epoch
  (1970-01-01T00:00:00Z), then saves `activitywatch` in the settings. All
  earlier history therefore stays with ActivityWatch.
- A new installation has no settings file. Onboarding appends its first
  selection, effective at the confirmation instant, before it saves the
  settings. A day or week that starts before confirmation reads its
  pre-confirmation part as unknown, and ActivityWatch is not read for it.
- If saved settings name a source but no selection row exists, WorkTime
  appends that source effective at the startup instant, never earlier.

Changing the source in settings appends a selection effective at the change
instant. Appending the source already in effect is a no-op, so each of these
steps can be retried after a crash.

### ManualAdjustment

Represents a user-owned correction without mutating the activity source's
records.

- `Id`: stable identifier
- `WorkdayDate`: local Monday–Friday date
- `Kind`: `add-work`, `exclude-time`, `reclassify`
- `StartUtc` / `EndUtc`: optional bounded interval
- `Duration`: optional duration form for added work
- `FromLabel` / `ToLabel`: optional classification change
- `Reason`: optional user text
- `CreatedAtUtc`
- `SupersedesId`: optional prior adjustment being corrected

Exactly one of an interval or an allowed duration-only form is required.
Adjustments that would double count existing credited coverage are rejected or
split into non-overlapping coverage before commit.

### CreditLedgerEntry

Explains positive overtime movement across a workweek.

- `Id`
- `WorkdayDate`
- `WeekStartDate`
- `Kind`: `earned`, `applied`, `expired`, `reversed`
- `DurationTicks`: positive magnitude stored at exact `TimeSpan` tick precision
- `OriginWorkdayDate`
- `OriginRevision`
- `CreatedAtUtc`

Ledger entries can be replaced transactionally by deterministic recomputation
from the earliest changed day through Friday. Negative under-time never creates
an `earned` entry. Optional weekly debt is not a ledger movement: surplus that
repays debt is simply not earned, and each summary stores the debt left at the
end of its day in `CarryDebtTicks` (zero when debt is off). `DebtPending` marks a
summary computed with debt on while its day was still running: its shortfall is
not in the debt yet, so the week stays on the historical closure worklist until a
later replay finishes the day.

Derived consumption uses the earliest earned credit first, with one movement
per earning day and durable revision. Applied credit is consumed before the
same day's new overtime is earned. Expired movements are materialized only
after the local Friday workday ends; Monday starts a new balance regardless
of whether the application was running when the previous week expired.

Whole-week commits compare all five expected latest daily revisions, including
missing days, before appending summaries and replacing the ledger in one
transaction. Ledger earned, applied, and actually expired totals must match the
selected summaries. Origins must reference those durable revisions, and total
consumption cannot exceed the corresponding earned lot. Unchanged movements
retain their identifiers and creation timestamps.

### NotificationReceipt

Prevents duplicate Windows notifications.

- `WorkdayDate`
- `Trigger`: `completion-approach`, `latest-departure-approach`, `day-complete`
- `EligibleAtUtc`
- `ReservedAtUtc`: schema v5 renames the former `DeliveredAtUtc` column
- `CombinedDeliveryId`: delivery identifier shared by merged triggers

Unique key: (`WorkdayDate`, `Trigger`).

The repository reserves each new trigger in one transaction before calling
Windows. A previously reserved trigger cannot be sent again, including after
restart or a send failure. A crash between reservation and Windows acceptance
can lose a warning; a reservation does not prove visible delivery.

### DailySummaryRevision

Caches an explainable computed result for history and change detection.

- `WorkdayDate`
- `Revision`
- `ComputedAtUtc`
- `TimezoneId`
- `InputFingerprint`
- credited, excluded, and unknown totals stored as exact duration ticks
- nominal and effective targets stored as exact duration ticks
- carry available, applied, earned, and expired stored as exact duration ticks
- projection and indicator state
- source quality and pending-review flags
- a versioned typed breakdown of accounting label, disposition, exact duration,
  optional UTC interval, and optional stable adjustment or review identifier

The summary is a cache, not source of truth. A new revision is written when
source evidence, settings, or adjustments affecting the day change.
Callers supply a nonblank input
fingerprint and contiguous per-day revision starting at 1. Appending is
idempotent for the same key and exact persisted content, rejects conflicting
content at an existing key, and never rewrites a stored revision. Inserting a
revision deletes the day's earlier revisions: every reader uses the latest
revision only, and a live day otherwise appended one revision per refresh
(issue #78). Schema 8 removes superseded revisions once and compacts the file. Legacy
breakdown JSON is preserved by migration but is rejected explicitly when it
does not match the versioned typed format. The store does not decide when to
write a revision or generate fingerprints.

### SchemaVersion

- `Version`
- `AppliedAtUtc`

Migrations are forward-only and transactional. Before destructive schema
changes, the release process must test backup and recovery.

## Preferences in JSON

`WorkTimeSettings` includes:

- nominal target and minimum effective target;
- lunch start/end;
- latest departure and orange margin;
- long-lock threshold;
- notification toggles and lead times;
- start-with-Windows choice;
- selected activity source, mirroring the latest `ActivitySourceSelection`
  (the ledger is authoritative for history); a settings file written before
  D-021 has no value and reads as ActivityWatch;
- built-in idle threshold, default 180 seconds, from 30 seconds to 30 minutes;
- ActivityWatch base URL and selected bucket;
- update-check toggle and last-check instant;
- language override;
- log limits.

The optional ActivityWatch API key is stored separately using current-user
DPAPI and is referenced only by a non-secret key identifier.

## Transient domain entities

### SourceInterval

Normalized interval with source, start/end UTC, state, freshness, and quality.
Sources include built-in activity spans or ActivityWatch AFK events, whichever
is selected for the instant, Windows session/power, tracking controls, and
manual adjustments.

### AccountedInterval

Non-overlapping interval classified as credited, excluded, provisional, or
unknown with an explainable label.

### DailySummary

Immutable in-memory result published atomically to tray, views, and notification
evaluation.

## Recalculation order

1. Find the earliest workday affected by a source/settings/adjustment change.
2. Normalize and account that day.
3. Rebuild credit from that day through Friday in date order.
4. Write new daily revisions and ledger entries in one transaction.
5. Publish only after commit succeeds.
