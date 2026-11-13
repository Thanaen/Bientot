# Feature Specification: Weekly schedule, leave and public holidays

**Feature Branch**: `004-weekly-schedule-days-off`

**Created**: 2026-11-13

**Status**: Draft, ready for planning review

**Input**: The owner asked whether Bientôt handles public holidays and days of
leave without hurting the weekly debt or skewing the figures, and for the best
way to handle them. A study (2026-11-12) found three flaws and recommended
« one target per day ». The owner then answered its questions: the working
days of the week must be configurable, part-time ships at the same time, the
holiday settings proposed in the study needed a better system, and there is
no list of upcoming leave. The design was then refined and reviewed by the
Designer, UX Writer and Screenshot reviewers until they agreed. It is ratified
as decision D-025, which replaces D-006.

## Background: what is wrong today

Checked on `main` at `124b531`:

1. **Credit is lost on a day off.** `calculate_week` applies the weekly credit
   to every weekday, worked or not. A finished day with no counted work
   creates no debt, but it still consumes up to « target − minimum » of
   credit (1 h with the defaults), which then expires unused.
2. **A day off with a little activity creates a debt.** Ten minutes on the PC
   on a public holiday leave a debt of 6 h 50 min. A half day of leave
   (3 h 30 min worked) leaves a debt of 3 h 30 min and the flyout says
   « En retard ».
3. **The week is fixed.** Monday to Friday is hard-coded
   (`LocalWorkday::is_working_day`, `credit::is_workday`), with a single
   target (D-006). Part-time work and Saturday work cannot be expressed.

## Scope

In scope: one target per day in the calculation core; a configurable weekly
schedule (working days Monday to Sunday, each with an optional target of its
own); leave and half days of leave declared on the days of the current week;
French public holidays computed offline, treated as holidays without any
confirmation, with exceptions remembered per holiday; the week view, flyout,
tray, palette, settings, onboarding, export and notifications that follow
from them.

Out of scope: leave planned beyond the current week, a list of upcoming
leave, leave balances, closures, working exceptionally on a non-working day,
calendars of other countries (they come later through `holidays` plugin
packs, D-024), calendars fetched from the network, a choice of morning or
afternoon for a half day.

## Vocabulary

French strings are the reference; English strings are proposals for the
string table, to be confirmed by the UX Writer at implementation.

| Concept | French | English (proposed) |
| --- | --- | --- |
| Day type: worked | « Journée travaillée » | "Working day" |
| Day type: half day of leave | « Demi-journée de congé » (« Demi-journée » where space is tight) | "Half day off" ("Half day") |
| Day type: day of leave | « Journée de congé » (« Congé » in rows) | "Day off" |
| Day type: public holiday | « Jour férié », row « Férié · <nom> » | "Public holiday", row "Holiday · <name>" |
| Day type: non-working weekday | « Jour non ouvré » (flyout), « Non ouvré » (rows, settings) | "Non-working day", "Non-working" |
| Umbrella term | « Sans travail prévu » (legend) | "No work planned" |
| Return to work | « Passer en journée travaillée » | "Make it a working day" |
| Settings group | « Semaine type » | "Weekly schedule" |
| Week total | « Objectif de la semaine » | "Weekly target" |
| Holiday settings | « Calendrier » (Aucun / France / France (Alsace-Moselle)), « Jours fériés travaillés », « Retirer » | "Calendar" (None / France / France (Alsace-Moselle)), "Holidays you work", "Remove" |
| Finished day without time | « Aucun temps compté », « Marquer comme congé » | "No time counted", "Mark as day off" |

« Jour off » is never used. « Non travaillé » keeps its current meaning: an
excluded slot in a timeline.

The full French copy, with typography (U+00A0 inside durations and before
« : ; ! ? »), is in the design hand-off: `conception.md` and
`maquettes/v2/copy.json` of the study folder (see `research.md`).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A day without work costs nothing (Priority: P1)

As a user who did not use the PC on a weekday (leave, illness, holiday), I
keep my weekly credit for the next worked day, and the week view does not
say I am missing 7 hours.

**Why this priority**: it fixes the costliest flaw for everyone, with no new
interface, and can ship on its own as a `fix(app)` before the rest.

**Independent Test**: with the defaults (7 h, minimum 6 h, credit on), work
9 h on Monday, nothing on Tuesday, 6 h on Wednesday; Wednesday is on target
and Tuesday shows « Aucun temps compté » in a neutral tone.

**Acceptance Scenarios**:

1. **Given** Monday earned 2 h of credit, **When** Tuesday finishes with no
   counted work, **Then** Tuesday applies no credit and Wednesday's effective
   target is 6 h.
2. **Given** a finished worked day with no counted work, **When** the week
   view shows it, **Then** its status is « Aucun temps compté » in a neutral
   tone, not « Il manque 7 h 00 min », and its details say that nothing was
   counted that day.
3. **Given** the same day once US3 is delivered, **When** its row is open,
   **Then** it offers « Marquer comme congé », which removes its target from
   the weekly target.
4. **Given** a finished day with some counted work below its target,
   **When** the week is calculated, **Then** debt and credit behave exactly
   as today (only the zero-work case changes).

---

### User Story 2 - My week as I work it (Priority: P1)

As a user who works part-time, on Saturdays, or shorter on Fridays, I set my
weekly schedule once and every figure (week view, flyout, tray colour,
departure, credit, debt, notifications) follows it.

**Why this priority**: the owner asked for configurable working days, and
part-time ships with it. Leave and holidays build on the same per-day target.

**Independent Test**: in **Paramètres › Horaires › Semaine type**, turn
Wednesday off and type 4:00 for Friday; the weekly target reads « 25 h 00 min
sur 4 jours » (with 7 h days), the current week recalculates from today, and
past days keep their targets.

**Acceptance Scenarios**:

1. **Given** a fresh install or an update, **When** the user opens
   **Paramètres › Horaires**, **Then** a « Semaine type » group sits between
   « Objectifs » and « Journée », with seven rows (lundi to dimanche), Monday
   to Friday on, Saturday and Sunday « Non ouvré », every target field empty.
   Nothing else changes for an existing user.
2. **Given** an empty target field on a working day, **When** it is shown,
   **Then** it displays the placeholder « 7:00 · par défaut » (the current
   « Objectif quotidien »), visibly distinct from a typed value in light and
   dark themes.
3. **Given** the user types a target equal to « Objectif quotidien »,
   **When** the setting is saved, **Then** the field goes back to empty, so
   the day keeps following the daily target.
4. **Given** a day with its own target, **When** the user turns it off and on
   again, **Then** its own target is kept.
5. **Given** the user turns off the last working day, **When** the change is
   attempted, **Then** it is refused with a message; at least one day stays
   on.
6. **Given** a day whose target is at or below « Objectif minimum »,
   **When** its row is shown, **Then** it carries the hint « À l'objectif minimum ou en
   dessous : le crédit ne s'y applique pas. » at the standard description
   size. When « Objectif quotidien » equals « Objectif minimum », the hint is
   shown once under the group instead of on every row.
7. **Given** any change to the schedule, the daily target or the minimum,
   **When** it is saved (immediately, « Paramètres enregistrés. »), **Then**
   it applies from today, today included; earlier days of the current week
   and closed weeks keep the targets and plans they had.
8. **Given** Saturday is a working day, **When** the week view shows the
   current week, **Then** it has six rows and is titled « Semaine du 9 au 14
   novembre »; a non-working weekday inside the range (for example Monday in
   a Tuesday-to-Saturday week) shows as « Non ouvré » with an outlined track
   and « — ».
9. **Given** today is a non-working day of the schedule, **When** the flyout
   opens, **Then** it shows « Jour non ouvré » as on a weekend today, with no
   departure and a neutral tray.
10. **Given** a new install, **When** onboarding reaches the « Horaires »
    step, **Then** it shows a « Jours ouvrés » row with seven day toggles,
    Monday to Friday on, each showing its state by a check mark as well as by
    colour and exposed as a toggle to screen readers, with the help line « Un
    objectif différent certains jours ? Il se règle ensuite dans Paramètres ›
    Horaires. »; per-day targets are only in the settings.

---

### User Story 3 - Declare leave in the current week (Priority: P2)

As a user taking a day or a half day off this week, I say so on that day and
the week, the departure and the flyout stop treating me as late.

**Why this priority**: with US1 a day off with the PC off is already right;
this story covers the days where the PC was used, and half days.

**Independent Test**: open Thursday of the current week, pick « Demi-journée
de congé » in the day-type selector; Thursday's target becomes 3 h 30 min,
the subtitle reads « … · 1 demi-journée », the departure no longer reserves
lunch, and the flyout shows « Demi-journée de congé ».

**Acceptance Scenarios**:

1. **Given** any day of the current week (past, today or to come) that has a
   target, **When** its row is open, **Then** a compact day-type selector
   under its status lines shows « Journée travaillée » and offers
   « Demi-journée de congé » and « Journée de congé », with a short note on
   what each does.
2. **Given** the user picks a type, **When** it applies, **Then** the row,
   the weekly target, the subtitle (« 10 h 40 min comptées sur 17 h 30 min ·
   1 congé, 1 férié, 1 demi-journée », with plurals « 2 congés, 2 fériés, 2
   demi-journées »), the departure and the tray recalculate at once, and the
   collapsed selector reads « Demi-journée » or « Congé ».
3. **Given** a day of leave that had counted time, **When** it is marked,
   **Then** the row says « 2 h 40 min d'activité de ce jour ne sont plus
   comptées. Elles restent enregistrées. » with « Annuler », with no
   confirmation dialog.
4. **Given** a day of leave, a holiday or a non-working day with activity,
   **When** its row or flyout is shown, **Then** it says « 3 h 10 min
   d'activité de ce jour ne sont pas comptées. Elles restent enregistrées. »
   next to « Passer en journée travaillée » (singular for one minute, see
   FR-024).
5. **Given** today is a day of leave, **When** the flyout opens, **Then** it
   is titled « Journée de congé », says « Aucun travail prévu », shows
   « Temps travaillé — » and offers « Passer en journée travaillée ».
6. **Given** today is a half day, **When** the flyout opens, **Then** it
   shows the departure against the halved target with the line
   « Demi-journée de congé », and no « En retard » state before that target.
7. **Given** the palette, **When** the user searches, **Then** it offers
   « Journée : marquer comme congé », « Journée : marquer comme demi-journée
   de congé » and « Journée : marquer comme travaillée » for the focused day
   of the current week.
8. **Given** a day of a closed week, **When** its row is open, **Then** no
   selector is offered.
9. **Given** any marking, **When** the user exports data, **Then** the
   `.bientot` file carries it, and an import restores it.

---

### User Story 4 - Public holidays without a list to manage (Priority: P2)

As a user in France, I see public holidays treated as days without work
automatically, and when I do work one I say so once and Bientôt remembers it
for the following years.

**Why this priority**: holidays are frequent (about ten per year on weekdays)
and the owner rejected a settings list of switches; this replaces it with
something that needs no setup.

**Independent Test**: in the week of 9 to 13 November 2026 with the France
calendar, Wednesday 11 shows « Férié · Armistice », the weekly target is
28 h; « Passer en journée travaillée » turns it back into a 7 h day, and
**Paramètres › Horaires › Jours fériés** lists Armistice under « Jours fériés
travaillés » with « Retirer ».

**Acceptance Scenarios**:

1. **Given** the France calendar, **When** a French public holiday falls on a
   working day of the schedule, **Then** it is a holiday from the start of
   its week: no target, nothing counted, no credit applied, no debt, no
   departure, no notification, and the row reads « Férié · <nom> ». There is
   no confirmation and no pending state.
2. **Given** a holiday row is open, **When** it is shown, **Then** it explains
   « Ce jour est férié (Armistice). Bientôt le compte comme tel, sans
   objectif. Si vous travaillez ce jour-là, passez-le en journée travaillée :
   ce choix sera retenu pour les années suivantes. » with a single button
   « Passer en journée travaillée » (the « aujourd'hui » variant when the
   holiday is today), and the day-type selector offers « Journée travaillée »,
   « Demi-journée de congé » and « Jour férié ».
3. **Given** the user makes a holiday a working day (button, selector or
   palette), **When** it applies, **Then** that day recounts from its start,
   and the holiday is added to « Jours fériés travaillés » so the same holiday
   is a working day in the following years too.
4. **Given** a holiday remembered as worked, **When** the user picks « Jour
   férié » on it in a later week, or « Retirer » in the settings, **Then** it
   is removed from the list and becomes a holiday again from the current week
   on (closed weeks do not change).
5. **Given** the user picks « Demi-journée de congé » on a holiday, **When**
   it applies, **Then** it is valid for that day only and does not change the
   remembered list.
6. **Given** a holiday that falls on a non-working day of the schedule,
   **When** the week is shown, **Then** the day stays « Non ouvré »; nothing
   is shown or remembered.
7. **Given** **Paramètres › Horaires**, **When** the user opens the
   « Jours fériés » group (directly under « Semaine type »), **Then** it has
   two rows: « Calendrier » (Aucun, France, France (Alsace-Moselle)) and
   « Jours fériés travaillés » (the remembered exceptions, each with
   « Retirer » and its tooltip « Ce jour redevient férié. », or « Aucun pour
   l'instant. »).
8. **Given** the calendar is set to « Aucun », **When** a French holiday
   comes, **Then** it is an ordinary day; the remembered list is kept for a
   later return to France.
9. **Given** the palette and a holiday in the current week, **When** the user
   searches, **Then** it also offers « Journée : marquer comme jour férié »
   on that holiday only.

### Edge Cases

- **Half day on a short day**: half of that day's target, rounded up to the
  minute (a 4 h Friday becomes 2 h).
- **Schedule changed mid-week**: applies from today; credit and debt already
  carried stay and continue on the remaining working days.
- **Today becomes non-working while time is already counted**: same as
  marking leave on a worked day (Acceptance US3-3).
- **Every remaining day of the week has no target**: the remaining credit
  expires at the end of the week (Sunday night), as it expires after Friday
  today; D-007 is otherwise unchanged and nothing carries to next week.
- **Credit earned on Saturday** is usable only on a working Sunday of the
  same week, otherwise it expires.
- **Recurring appointment on a day without target**: ignored that day, as on
  a weekend today.
- **Long-lock reviews and « à régler » slots on a day without target**: not
  raised, as on a weekend today; they come back if the day becomes worked.
- **Corrections on a day without target**: refused as on a weekend today; a
  day must be worked to be corrected.
- **Debt cap**: unchanged and not prorated for short weeks.
- **Daylight saving and midnight**: unchanged; days spanning midnight stay out
  of scope (D-006 rule kept by D-025).
- **Clock or time zone change across a holiday**: the holiday is a civil date
  in the local calendar, like every workday.
- **Alsace-Moselle**: adds Vendredi saint (Good Friday) and 26 décembre
  (Saint-Étienne) to the eleven national holidays.
- **Remembered holiday whose date moves** (Easter-based): remembered by
  holiday identity, not by date.
- **Import of an older `.bientot` file**: no plans, no schedule history; the
  imported settings' schedule applies as a single version since forever.

## Requirements *(mandatory)*

### Functional Requirements

**Day plans and targets**

- **FR-001**: Every civil date MUST resolve to exactly one day plan:
  *worked* (target = the day's target), *half day of leave* (target = half of
  the day's target, rounded up to the minute), *leave*, *holiday* or
  *non-working* (no target).
- **FR-002**: The plan of a date MUST be resolved in this order: an explicit
  marking of that date (US3, US4), else a built-in holiday of the selected
  calendar on a working day of the schedule that is not remembered as worked,
  else the weekly schedule version in force on that date.
- **FR-003**: A day without a target MUST count nothing: no credited work, no
  credit applied or earned, no debt, no departure, no colour state, no
  notification, no review, no recurring appointment. Its activity MUST stay
  recorded, so that turning it into a worked day recounts it from its start.
- **FR-004**: « Compté » MUST only ever contain real work. No time is
  fabricated for leave or holidays.

**Weekly credit and debt**

- **FR-005**: Weekly credit MUST apply only to days with a target, at most
  « day target − Objectif minimum » per day and never below zero, so it never
  lowers any day's target below the minimum.
- **FR-006**: A finished day with a target and zero counted work MUST apply no
  credit and incur no debt (US1). Its target still counts in the weekly
  target unless it is marked as leave.
- **FR-007**: Debt MUST be computed against the day's target after credit, for
  a finished day with a target and some counted work, as today.
- **FR-008**: Credit and debt MUST end with the week on Sunday night; the
  weekly calculation MUST accept the seven days of the week. The debt cap is
  unchanged.
- **FR-009**: Surplus on a day beyond its target after credit MUST first repay
  debt, then become credit, as today.

**Weekly schedule**

- **FR-010**: Settings MUST hold a weekly schedule: for each weekday, Monday to
  Sunday, whether it is a working day and an optional own target (0:01 to
  24:00). An empty target follows « Objectif quotidien ». Default: Monday to
  Friday on, no own targets.
- **FR-011**: At least one day MUST stay a working day; a target equal to the
  daily target MUST be stored as empty; an own target MUST be kept when the day
  is toggled off and on.
- **FR-012**: Changes to the schedule, « Objectif quotidien » and « Objectif
  minimum » MUST apply from the local date of the change, today included. The
  settings MUST keep dated versions so that any past date resolves to the
  targets in force that day, deterministically, after a restart or an import.
- **FR-013**: « Objectif minimum » MUST stay at or below « Objectif
  quotidien »; it MAY be above a day's own target (a « jour court »). Its
  description becomes « Le crédit hebdomadaire ne fait jamais descendre
  l'objectif d'un jour sous ce minimum. Heures et minutes, par exemple
  6:00. »
- **FR-014**: The « Semaine type » group MUST show « Objectif de la semaine »
  with the sum of the targets and the number of working days (« 25 h 00 min
  sur 4 jours »), and the sentence « Un changement s'applique à partir
  d'aujourd'hui. Les jours passés gardent leur objectif. »
- **FR-015**: Onboarding MUST add the « Jours ouvrés » row to its « Horaires »
  step (US2-10).

**Forecast**

- **FR-016**: The departure forecast MUST use the day's target after credit.
- **FR-017**: The forecast MUST reserve the lunch pause as today (when the
  projected departure crosses the lunch start), except on a half day of leave
  and on a « jour court » (own target strictly below « Objectif minimum »),
  which never reserve it. The details of such a day MUST say « Pause déjeuner
  non réservée dans la prévision. »

**Leave**

- **FR-018**: The user MUST be able to set the plan of any day of the current
  week that has a target, or is a holiday, to *worked*, *half day of leave*,
  *leave* or (on a built-in holiday) *holiday*, from the week view selector,
  from the flyout (« Passer en journée travaillée ») and from the palette.
- **FR-019**: Plan markings MUST be stored durably, append-only, with undo
  (« Annuler ») restoring the previous plan, and MUST be carried by export and
  import.
- **FR-020**: Days of closed weeks MUST NOT be marked; there is no list of
  upcoming leave.

**Holidays**

- **FR-021**: Bientôt MUST compute offline the French public holidays: 1er
  janvier, Lundi de Pâques, 1er mai, 8 mai, Ascension, Lundi de Pentecôte,
  14 juillet, 15 août, 1er novembre, 11 novembre, 25 décembre; and for
  « France (Alsace-Moselle) » also Vendredi saint and 26 décembre.
- **FR-022**: Settings MUST hold the calendar (Aucun, France, France
  (Alsace-Moselle)) and the set of holidays remembered as worked, by holiday
  identity. Default calendar: France when the Windows region is France,
  otherwise Aucun.
- **FR-023**: Making a built-in holiday a working day MUST add it to the
  remembered set; picking « Jour férié » on it or « Retirer » MUST remove it.
  These changes apply from the current week; closed weeks keep their plans.

**Display**

- **FR-024**: Rows and flyouts of days without a target that have activity
  MUST state the uncounted time: « <durée> d'activité de ce jour ne sont pas
  comptées. Elles restent enregistrées. », in the singular (« 1 min
  d'activité de ce jour n'est pas comptée. Elle reste enregistrée. ») for
  exactly one minute only; right after marking leave, « ne sont plus
  comptées » with « Annuler » (exact copy: `contracts/copy.md`).
- **FR-025**: The week view MUST show Monday to Friday plus Saturday and Sunday
  when they are working days of the schedule in force that week, with the
  title spanning the shown days; the legend swatch for days without work
  reads « Sans travail prévu ».
- **FR-026**: The weekly target in the week header MUST be the sum of the day
  targets before credit; non-working days are not mentioned in the subtitle.
- **FR-027**: Every new text MUST exist in French and English in the string
  table; durations use the app's formatter (U+00A0).

**Data and privacy**

- **FR-028**: Everything stays local; no network access is added.
- **FR-029**: Diagnostics MUST NOT export holiday names chosen or leave dates
  beyond what the existing summaries already hold.

### Key Entities

- **WeeklySchedule**: seven `ScheduledDay` (working, own target), with the
  daily target and the minimum it was saved with.
- **ScheduleVersion**: a WeeklySchedule plus the date it applies from.
- **DayPlan**: Worked, HalfDayLeave, Leave, Holiday(HolidayId), NonWorking; with
  the resolved target.
- **DayPlanMarking**: an explicit plan for a date of the current week, with its
  revision and operation key.
- **HolidayCalendar**: None, France, FranceAlsaceMoselle; yields
  `(Date, HolidayId)` for a year.
- **HolidayId**: a stable identity of a holiday, independent of its date.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In the reference scenarios of `quickstart.md`, no day without a
  target consumes credit or creates debt (0 min in both, checked by tests).
- **SC-002**: A user with the default settings who changes nothing sees the
  same targets, credit, debt and departures as before on every worked day
  (regression suite of the existing credit and forecast tests unchanged).
- **SC-003**: A French user goes through a year of holidays with zero setup
  actions, and at most one action per holiday they work.
- **SC-004**: Declaring a day of leave takes at most two clicks from the open
  day row, and undoing it one.
- **SC-005**: Changing the schedule never changes a figure of a closed week
  (property test over random schedules and changes).
- **SC-006**: The refresh stays within the existing performance gates; the
  holiday computation for a year takes under 1 ms.

## Assumptions

- The defaults stay 7 h and 6 h; examples use them.
- The France default from the Windows region is the one remaining guess. It
  only selects holidays that are already holidays for nearly every French
  employee; the owner may still change it.
- A half day has no morning or afternoon; the forecast simply does not reserve
  lunch.
- Working exceptionally on a non-working day is done by editing the schedule;
  a dedicated action may come later.
- Holidays of other countries come through plugins (D-024) and feed the same
  `HolidayCalendar` boundary.
