# Research: Weekly schedule, leave and public holidays

## Sources

- Study of 2026-11-12 (`etude.md`) and design of 2026-11-13
  (`conception.md`), with mockups and generator in `maquettes/v2/`, kept in
  the project files under `bientot/conges-feries/`. The mockups are the visual
  reference; the screens to build are listed in `tasks.md`.
- Code read on `main` at `124b531`: `credit/policy.rs`, `credit/mod.rs`,
  `time.rs`, `configuration/mod.rs`, `forecasting/departure.rs`,
  `accounting/engine.rs`, `bientot-core/src/{accounting,weekly}.rs`,
  `bientot-store/src/database/schema.rs`, `bientot-app/src/{week,workday,live}.rs`.
- Decisions D-003 (lunch work is credited), D-006 (fixed week, replaced),
  D-007 (credit), D-015 (forecast), D-024 (holiday packs).

## R1. Measured flaws

A throwaway test of `calculate_week` (Mon 9 h, Tue 0, Wed 7 h, Thu 6 h, 7 h
target, 6 h minimum, credit and debt on) showed: Tuesday applied 60 min of
credit with no work; a holiday with 10 min of work left 410 min of debt; a
half day of 3 h 30 left 210 min of debt. Decision: one target per day (study
option C). Rejected: fixing the engine only (A, half days stay wrong),
counting leave as work (B, fabricates time, breaks Principle II), a weekly
target (D, loses daily colour and departure).

## R2. Working days and part-time

The owner asked for configurable working days and agreed to ship part-time
at the same time. Decision: a « Semaine type » of seven days, each working or
not with an optional own target. Rejected: a separate part-time setting
(two concepts for one need), a weekly hour total split evenly (wrong for a
short Friday).

## R3. Credit on short days

Decision: one floor rule for every day: credit applied ≤ day target −
minimum, never negative. It replaces the study's special case « half day
applies no credit » and covers short days. A part-timer with every day at or
below the minimum earns credit that never applies; the settings row and the
day details say so (« À l'objectif minimum ou en dessous : le crédit ne s'y applique
pas. »).

## R4. Holidays

The owner rejected the study's system (calendar switched on from the Windows
region, one switch per holiday, Lundi de Pentecôte worked by default). Six
options were compared in the design; the reviewers chose F after rejecting E:

| Option | Outcome |
| --- | --- |
| A. Region default + per-holiday switches | Rejected by the owner. |
| B. Calendar chosen once, no exceptions | Wrong every year for anyone who works the « journée de solidarité ». |
| C. Holidays as ordinary leave | Typing for something computable; no holiday name. |
| D. Detection from absence (US1 only) | 10 min of PC use recreates debt; target stays 7 h. |
| E. Treated as holiday, confirmed in place the first time | Rejected in review: the confirmation changes nothing for the common case and adds ten pending badges a year. |
| **F. Treated as holiday silently; only exceptions remembered** | Chosen. No setup, one action per holiday actually worked, remembered for later years. |

Default calendar: France when the Windows region is France, else Aucun. It
only selects days that are holidays for nearly every French employee; it is
listed as the remaining open point for the owner.

## R5. Lunch on short days

D-003 credits active work inside the lunch window, so a short day can end
inside it. Changing the reservation rule for every day would move the
departure of current users who arrive early. Decision: keep today's rule,
except that a half day of leave and a « jour court » (own target strictly
below the minimum) never reserve the pause; the details say so. The visible
jump at the minimum (5:59 vs 6:00) was judged acceptable by the Designer.

## R6. Past days

Decision: dated settings versions (schedule, daily target, minimum) and the
plan persisted in each summary revision. A change applies from today. This
also fixes an existing gap: today a change of the daily target rewrites the
earlier days of the current week.

## R7. Easter

Anonymous Gregorian algorithm (Meeus/Jones/Butcher), integer arithmetic,
valid for every Gregorian year; checked against a fixed table 2024–2035
(Easter 2026-04-05, 2027-03-28, 2028-04-16, 2029-04-01, 2030-04-21).

## R8. Review

Design and wording: Designer, UX Writer and Screenshot reviewers, three
rounds on the design of 2026-11-13, all approved (see `conception.md`,
section Relecture). The English strings of `contracts/copy.md` are proposals
to put to the UX Writer with the first UI pull request.
