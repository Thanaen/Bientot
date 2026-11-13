# Data Model: Weekly schedule, leave and public holidays

Types live in `bientot-domain` unless noted. Durations are `SignedDuration`
in code and whole minutes in storage.

## ScheduledDay

| Field | Type | Rule |
| --- | --- | --- |
| `working` | bool | |
| `target` | Option<duration> | 0:01 to 24:00; `None` follows the daily target; a value equal to the daily target is normalised to `None` on save |

## WeeklySchedule

Seven `ScheduledDay`, indexed by weekday, Monday first. At least one
`working`. Default: Monday to Friday working, Saturday and Sunday not, no own
targets.

## ScheduleVersion

| Field | Type | Rule |
| --- | --- | --- |
| `since` | Option<Date> | `None` only for the oldest version; strictly increasing |
| `schedule` | WeeklySchedule | |
| `daily_target` | duration | the « Objectif quotidien » in force |
| `minimum_target` | duration | the « Objectif minimum » in force, ≤ `daily_target` |

The current version is assembled from `Settings.weekly_schedule`,
`nominal_daily_target`, `minimum_effective_target` and
`Settings.schedule_since`. Earlier versions are `Settings.schedule_history`,
at most 100, oldest first.

## HolidayId

A stable identity, independent of the date:

| Id | French name | Date |
| --- | --- | --- |
| `new_year` | Jour de l’An | 1 January |
| `good_friday` | Vendredi saint | Easter − 2 (Alsace-Moselle only) |
| `easter_monday` | Lundi de Pâques | Easter + 1 |
| `labour_day` | Fête du Travail | 1 May |
| `victory_1945` | Victoire 1945 | 8 May |
| `ascension` | Ascension | Easter + 39 |
| `whit_monday` | Lundi de Pentecôte | Easter + 50 |
| `bastille_day` | Fête nationale | 14 July |
| `assumption` | Assomption | 15 August |
| `all_saints` | Toussaint | 1 November |
| `armistice` | Armistice | 11 November |
| `christmas` | Noël | 25 December |
| `st_stephen` | Saint-Étienne | 26 December (Alsace-Moselle only) |

Names are in the string table (French and English). Unknown ids read from
settings (a later version or a plugin pack) are kept and ignored.

## HolidayCalendar

`None` | `France` | `FranceAlsaceMoselle`. `holidays(year) -> [(Date,
HolidayId)]`, cached per year. Easter: anonymous Gregorian algorithm.

## DayPlanKind and DayPlan

`DayPlanKind`: `Worked`, `HalfDayLeave`, `Leave`, `Holiday`, `NonWorking`.

`DayPlan`:

| Variant | Target | Shown as |
| --- | --- | --- |
| `Worked { target }` | the day's target | « Journée travaillée » |
| `HalfDayLeave { target }` | ⌈day's target / 2⌉ to the minute | « Demi-journée de congé » |
| `Leave` | none | « Journée de congé » / « Congé » |
| `Holiday { id }` | none | « Jour férié » / « Férié · <nom> » |
| `NonWorking` | none | « Jour non ouvré » / « Non ouvré » |

`DayPlan::target() -> Option<duration>`, `is_short(minimum) -> bool` (own
target strictly below the minimum, for the lunch exception),
`credit_cap(minimum) -> duration` (`(target − minimum).max(0)`, zero without
target).

## Settings (additions)

| JSON key | Type | Default |
| --- | --- | --- |
| `weeklySchedule` | array of 7 `{ "working": bool, "target": "H:mm" \| null }`, Monday first | Monday–Friday working, no targets |
| `scheduleSince` | date \| null | null |
| `scheduleHistory` | array of `{ "since": date \| null, "weeklySchedule": …, "nominalDailyTarget": …, "minimumEffectiveTarget": … }` | `[]` |
| `holidayCalendar` | `"none"` \| `"france"` \| `"france-alsace-moselle"` | `"france"` when the Windows region is FR on first read, else `"none"`; written on first save |
| `workedHolidays` | array of HolidayId strings | `[]` |

Validation (`configuration/validator.rs`): seven entries, at least one
working, targets in range, history `since` strictly increasing and before
`scheduleSince`, every version's minimum ≤ its daily target.

## DayPlanMarking (store, migration 12)

```sql
CREATE TABLE DayPlanMarking (
    WorkdayDate TEXT NOT NULL,
    Revision INTEGER NOT NULL,
    Kind TEXT NOT NULL CHECK (Kind IN ('worked', 'half-day-leave', 'leave', 'holiday', 'cleared')),
    RecordedAtUtc TEXT NOT NULL,
    OperationKey TEXT NULL,
    PRIMARY KEY (WorkdayDate, Revision)
);
CREATE UNIQUE INDEX UX_DayPlanMarking_OperationKey
    ON DayPlanMarking (OperationKey) WHERE OperationKey IS NOT NULL;
ALTER TABLE DailySummaryRevision ADD COLUMN DayPlan TEXT NULL;
```

- The latest revision of a date is its marking; `cleared` means none.
- « Annuler » appends the previous kind (or `cleared`).
- Writes are refused for dates outside the current week (FR-020).
- `DailySummaryRevision.DayPlan` holds `worked`, `half-day-leave`, `leave`,
  `holiday:<id>` or `non-working`; `NULL` on rows written before migration
  12 reads as `worked` for Monday–Friday.

## DailyCreditInput (changed)

Adds `target: Option<SignedDuration>` and `credit_cap: SignedDuration`, both
resolved per day from the version in force that day, so a mid-week change of
the minimum is honoured. `calculate_week` no longer reads targets from
`Settings`; it keeps reading the credit and debt switches and the debt cap.
`DailyCreditResult::nominal_target` becomes the day's target (zero without
target).

## Week (app view model)

`Week.days` holds Monday–Friday plus Saturday and Sunday when working in the
version in force that week; each day carries its `DayPlan`, its uncounted
activity duration (for FR-024) and whether it can be marked (current week).
