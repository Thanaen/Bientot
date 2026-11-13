# Implementation Plan: Weekly schedule, leave and public holidays

**Branch**: `004-weekly-schedule-days-off` | **Date**: 2026-11-13 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/004-weekly-schedule-days-off/spec.md`

## Summary

Replace the fixed Monday-to-Friday week with one target per day. A pure
resolver in `bientot-domain` turns a date into a `DayPlan` from three inputs:
the dated weekly schedule in the settings, the built-in French holiday
calendar with the holidays remembered as worked, and the explicit markings of
the current week stored in the ledger. Every place that asks
`LocalWorkday::is_working_day()` today asks the resolved plan instead, and
that method is deleted. `calculate_week` takes a target per day over seven
days and never applies credit to a day without target. The week view, flyout,
tray, palette, settings and onboarding follow.

Delivery is four vertical slices that can ship in one release, in this
order: US1 (engine fix, `fix(app)`), US2 (schedule), US3 (leave), US4
(holidays). US1 needs none of the new types and ships first.

## Technical Context

**Language/Version**: Rust 1.97, edition 2024 (`rust-toolchain.toml`)

**Primary Dependencies**: existing `jiff` (civil dates, weekdays), `gpui-pre`,
`rusqlite`, `serde_json`. No new dependency: the Easter date is computed with
the anonymous Gregorian algorithm (about 15 lines).

**Storage**:
- `settings.json` gains `weeklySchedule`, `scheduleHistory`, `holidayCalendar`
  and `workedHolidays` (`data-model.md`), read and written by
  `crates/bientot-store/src/settings/json.rs`; `schemaVersion` stays 1 because
  every new key is optional with a default equal to today's behaviour.
- The ledger database gains migration 12: table `DayPlanMarking`
  (append-only) and column `DayPlan` on `DailySummaryRevision`.
- The `.bientot` transfer file copies the database and the settings, so it
  carries both without a format change; a test proves it.

**Testing**: `cargo test --workspace` on Linux. Unit tests for the resolver,
the holiday calendar (2024–2035 against a fixed table), `calculate_week` over
seven days, the floor rule, the lunch exception; property tests that a
schedule change never alters a closed week (SC-005) and that the default
schedule reproduces today's results (SC-002); store tests for migration 12
and markings; view-model tests in `bientot-app`; `bientot-screenshots`
scenes for every new screen (FR/EN, light/dark).

**Target Platform**: Windows 11 x64; non-Win32 crates build and test on Linux.

**Project Type**: desktop tray application, Cargo workspace.

**Performance Goals**: the refresh stays within the current gates. Plan
resolution is a lookup; the holiday table for a year is computed on demand
and cached per year.

**Constraints**: local-first (no network), deterministic core (injected
clock and zone), existing size gate, French and English texts, design-system
artifacts updated with each UI change (project rule).

**Scale/Scope**: about 20 call sites of `is_working_day`, one engine
function, one migration, four settings rows, one selector, one flyout state,
three palette commands, one onboarding row.

## Constitution Check

| Principle | Check |
| --- | --- |
| I. Local-first privacy | Pass. Holidays are computed locally; no new data leaves the machine; markings hold dates and kinds only. |
| II. Explainable time accounting | Pass. Every day shows its plan and why it has no target; uncounted activity is stated (FR-024); the credit floor and the lunch exception are shown in the details. |
| III. Tray-first calm UX | Pass. No notification is added; holidays need no confirmation; leave is declared where the day is shown. |
| IV. Resilient activity source boundary | Not affected. Activity on days without target stays recorded by the source. |
| V. Simple, deterministic core | Pass. The resolver and calendar are pure functions of date, settings and markings; all inputs are injectable. |
| VI. Contained extensions | Pass. The calendar boundary is where future `holidays` plugin packs plug in (D-024); no plugin code is involved now. |
| Workflow 2: accounting decisions block implementation | Resolved by D-025 (added with this specification). |

## Project Structure

### Documentation (this feature)

```text
specs/004-weekly-schedule-days-off/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── calculation-rules.md
│   └── copy.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
crates/bientot-domain/src/
├── schedule.rs            # new: WeeklySchedule, ScheduleVersion, DayPlan, resolve()
├── schedule/holidays.rs   # new: HolidayCalendar, HolidayId, French dates, Easter
├── schedule/tests.rs
├── time.rs                # LocalWorkday::is_working_day deleted
├── configuration/mod.rs   # Settings: weekly_schedule, schedule_history, holiday_calendar, worked_holidays
├── configuration/validator.rs
├── credit/{mod,policy,ledger,entry}.rs  # per-day target, seven days, Sunday expiry
├── forecasting/departure.rs             # lunch exception
├── summaries/builder.rs                 # plan in the summary
└── notifications.rs

crates/bientot-core/src/
├── accounting.rs          # plan from the context instead of is_working_day
├── weekly.rs              # seven days, per-day targets, markings in the context
├── settings.rs            # dated versions on save
├── plans.rs               # new: read/append markings, undo
├── reviews.rs, tracking.rs, notifications.rs

crates/bientot-store/src/
├── database/schema.rs     # migration 12
├── plans.rs               # new: DayPlanMarking rows
├── settings/json.rs       # new keys
├── history.rs, credit.rs, adjustments.rs, transfer.rs (test only)

crates/bientot-app/src/
├── week.rs, week/details.rs, week_view/days.rs, week_view/details.rs  # rows, selector, subtitle, legend
├── workday.rs, flyout/body.rs, tray_model.rs  # day-off states
├── settings.rs, settings_view.rs              # Semaine type, Jours fériés
├── onboarding.rs, onboarding_view.rs          # Jours ouvrés
├── palette.rs                                 # three or four commands
├── live.rs, host.rs, automatic.rs             # plan instead of working_day
├── strings/table.rs                           # FR/EN texts
└── bin/screenshots/                           # scenes
```

**Structure Decision**: the plan is a domain concept, so its types and the
resolver live in `bientot-domain` beside `configuration` and `credit`. The
core captures settings and markings in `CalculationContext`, so one refresh
sees one consistent set of plans. The app never resolves plans on its own; it
reads them from summaries and from a core query for the current week.

## Design notes

### Plan resolution

`resolve(date, &Settings, marking: Option<DayPlanKind>) -> DayPlan`:

1. The schedule version in force on `date` (latest `since <= date`, else the
   oldest) gives `working` and the day's target (own target or the version's
   daily target).
2. If `marking` is present it wins (Worked, HalfDayLeave, Leave, Holiday),
   except that *Holiday* is only valid on a built-in holiday date.
3. Else if the calendar has a holiday on `date`, the day is working, and the
   holiday is not in `worked_holidays`, the plan is `Holiday(id)`.
4. Else Worked with the day's target, or NonWorking.

A marking of *Worked* on a non-working weekday is refused (out of scope,
spec Assumptions). `worked_holidays` is applied as it stands now to the
current week only; closed weeks are already persisted with their plan in
`DailySummaryRevision.DayPlan` and are not recalculated with new settings
(`close_historical_week` keeps using the persisted plan).

### Dated settings

`scheduleHistory` holds every earlier `ScheduleVersion` with its `since`; the
current one is `weeklySchedule` plus `nominalDailyTarget` and
`minimumEffectiveTarget`, with its own `since`. Saving any of the three on
date D:
- if the current version's `since` is D, replace it;
- else push the current version to history and start a new one since D.
History keeps at most 100 versions (oldest merged). A version without `since`
applies to every date before the next one, which is how an update and an old
import start.

### Credit

`DailyCreditInput` gains `target: Option<SignedDuration>` and `credit_cap`
(both from the plan and the version in force that day). `calculate_week`:
- accepts one to seven consecutive days from Monday;
- `applied = if target is None or work is zero and finished: 0 else
  balance.min(credit_cap)`;
- no surplus, no debt on a day without target;
- `credit_expiring_after_friday` becomes `credit_expiring_at_week_end` on the
  last day of the week (Sunday).
`credit::is_workday` is deleted; `ledger.rs` and `entry.rs` accept any day of
the week with a target.

US1 alone is the line `work is zero and finished => applied = 0` plus the
neutral status; it does not need the other changes.

### Weekly recalculation and history

`WeekPlan::workday_dates` returns the seven dates; every day is summarized
and persisted with its plan, days without target with targets 0. The current
special case that summarizes a weekend with `build_nominal` disappears. The
summary fingerprint includes the plan, so changing a plan or the settings for
the current week appends a new revision.

### Forecast

`project_departure` receives the plan; it skips the lunch reservation when
the plan is HalfDayLeave or the day's own target is strictly below the
minimum (« jour court »).

### Week view

Rows: Monday to Friday, plus Saturday and Sunday when they are working in the
version in force that week. The day-type selector lives under the status lines
of the open row, fixed min-width, popup offset 5 px under its trigger. The
holiday explanation block and the uncounted-time line are new parts of the
open row. The legend swatch for days without work reads « Sans travail prévu ».

### Settings view

Page `Schedule` gains « Semaine type » (seven rows at about 75 % of the
standard row height, switch + H:mm field with placeholder « 7:00 · par
défaut », hint for days at or below the minimum, « Objectif de la semaine »
line and sentence) between « Objectifs » and « Journée », then « Jours
fériés » (« Calendrier » select, « Jours fériés travaillés » list with
« Retirer »). The mockups in the hand-off are the reference.

## Complexity Tracking

| Choice | Why | Simpler alternative rejected because |
| --- | --- | --- |
| Dated schedule versions | Closed weeks and earlier days of the week must not move (FR-012, SC-005) | Recalculating with current settings would rewrite the current week's past days and confuse credit already shown. |
| Seven-day weekly calculation | Saturday and Sunday work (US2) | A Monday-to-Friday engine with a weekend patch would need two credit paths. |
| Append-only markings table | Undo, export and the ledger's audit style | A settings list would mix per-day facts into preferences and lose history. |
