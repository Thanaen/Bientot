---

description: "Task list for the weekly schedule, leave and public holidays"
---

# Tasks: Weekly schedule, leave and public holidays

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`

**Tests**: required. This feature changes accounting semantics (constitution,
Principle II and « Product and quality constraints »).

Each task names its files. **[P]** tasks touch different files and can run in
parallel. Each pull request follows `CONTRIBUTING.md`: Conventional Commits
with a scope (`fix(app)`, `feat(app)`, `test(app)`, `docs(repo)`), the « En
bref » / « In short » release lines for every `feat(app)`, `fix(app)` and
`perf(app)` pull request, French and English texts, and the design-system and
mockup artifacts updated with any UI change (project rule). Suggested pull
requests are marked **PR**.

## Phase 1: User Story 1 — a day without work costs nothing (P1) 🎯 MVP

**PR 1** `fix(app): keep the weekly credit for a day without counted work`.
Independent of every other phase.

- [ ] T001 [US1] In `crates/bientot-domain/src/credit/policy.rs`, apply no credit to a finished day with zero gross credited work; add the reference weeks W1 of `contracts/calculation-rules.md` and a regression test that a day with some work is unchanged, in `crates/bientot-domain/src/credit/tests.rs`.
- [ ] T002 [US1] In `crates/bientot-app/src/week.rs` (`day_status`) and `crates/bientot-app/src/week/details.rs`, show « Aucun temps compté » in a neutral tone for a finished day with a target and zero counted work, instead of « Il manque … »; FR/EN strings in `crates/bientot-app/src/strings/table.rs`; view-model tests in `crates/bientot-app/src/week/tests.rs`.
- [ ] T003 [US1] Add a screenshot scene (week with an empty Tuesday) to `crates/bientot-app/src/bin/screenshots/`; update the GPUI mockups artifact; « En bref » lines.

**Checkpoint**: US1 acceptance scenarios 1, 2 and 4 pass; scenario 3 waits for US3.

## Phase 2: Foundational — day plans (blocks US2, US3, US4)

**PR 2** `refactor(app): resolve a plan for every day` (no behaviour change).

- [ ] T010 [P] Create `crates/bientot-domain/src/schedule.rs` with `ScheduledDay`, `WeeklySchedule` (default Monday–Friday), `ScheduleVersion`, `DayPlanKind`, `DayPlan` and `resolve()` per `data-model.md` and `plan.md` « Plan resolution »; tests in `crates/bientot-domain/src/schedule/tests.rs`.
- [ ] T011 [P] Create `crates/bientot-domain/src/schedule/holidays.rs`: `HolidayId`, `HolidayCalendar`, Easter, `holidays(year)` for France and Alsace-Moselle, per-year cache; tests against a fixed table 2024–2035.
- [ ] T012 Add `weekly_schedule`, `schedule_since`, `schedule_history`, `holiday_calendar` (default `None` in this PR), `worked_holidays` to `Settings` in `crates/bientot-domain/src/configuration/mod.rs` and their validation in `configuration/validator.rs`; JSON keys in `crates/bientot-store/src/settings/json.rs` with round-trip and old-file tests.
- [ ] T013 Add migration 12 (`DayPlanMarking`, `DailySummaryRevision.DayPlan`) to `crates/bientot-store/src/database/schema.rs`; update the migration tests in `database/tests.rs` (the .NET parity list ends at 11: document why 12 has no .NET twin).
- [ ] T014 Add `crates/bientot-store/src/plans.rs` (append, latest per date, undo, current-week guard) and `crates/bientot-core/src/plans.rs`; capture the current week's markings in `CalculationContext` (`crates/bientot-core/src/accounting.rs`).
- [ ] T015 Replace every `LocalWorkday::is_working_day()` call (`bientot-core`: accounting, tracking, reviews; `bientot-store`: adjustments; `bientot-domain`: adjustments, history/revision, notifications; `bientot-app`: week, selection, automatic, host, workday, live) with the resolved plan's `target().is_some()`, then delete the method from `crates/bientot-domain/src/time.rs` and `credit::is_workday` from `credit/mod.rs`.
- [ ] T016 Persist the plan in each summary revision (`crates/bientot-domain/src/summaries/builder.rs`, `crates/bientot-store/src/history.rs`) and include it in the summary fingerprint (`crates/bientot-core/src/weekly.rs`).
- [ ] T017 Property test in `crates/bientot-core/src/weekly.rs` tests: with default settings and no markings, every figure equals the pre-refactor result over random weeks (SC-002).

**Checkpoint**: the workspace builds and every existing test passes unchanged.

## Phase 3: User Story 2 — my week as I work it (P1)

**PR 3** `feat(app): set the working days and the target of each day`.

- [ ] T020 [US2] Generalise `calculate_week` in `crates/bientot-domain/src/credit/policy.rs` to one to seven days, per-day `target` and `credit_cap`, no credit/surplus/debt without target; rename `credit_expiring_after_friday` to `credit_expiring_at_week_end`; adapt `ledger.rs` and `entry.rs`; reference weeks W4–W8 of `contracts/calculation-rules.md`.
- [ ] T021 [US2] In `crates/bientot-core/src/weekly.rs`, recalculate the seven days of the week with their plans, persist all of them, remove the weekend `build_nominal` special case, expire credit after Sunday; keep closed weeks on their persisted plans; property test SC-005 (a settings change never alters a closed week).
- [ ] T022 [US2] Dated settings in `crates/bientot-core/src/settings.rs`: saving the schedule, daily target or minimum on date D replaces or starts the version since D (`plan.md` « Dated settings »); cap at 100 versions; tests for mid-week changes (W6).
- [ ] T023 [US2] Forecast: pass the plan to `project_departure` in `crates/bientot-domain/src/forecasting/departure.rs`; skip the lunch reservation for a half day and a « jour court »; tests at 5:59/6:00 and with an early arrival on a full day (unchanged).
- [ ] T024 [US2] Settings view: « Semaine type » group in `crates/bientot-app/src/settings.rs` and `settings_view.rs` (seven rows, switch, H:mm field with placeholder « 7:00 · par défaut », normalisation, kept own target, last-day refusal, minimum hint, « Objectif de la semaine », sentence) between « Objectifs » and « Journée »; new « Objectif minimum » description; tests in `crates/bientot-app/src/settings/tests.rs`.
- [ ] T025 [US2] Week view: rows for Saturday and Sunday when working, « Non ouvré » rows with outlined track and « — », title spanning the shown days, legend « Sans travail prévu », minimum hint and lunch line in the details (`crates/bientot-app/src/week.rs`, `week/details.rs`, `week_view/days.rs`, `week_view/details.rs`).
- [ ] T026 [US2] Flyout and tray: a non-working day today behaves like a weekend today (`crates/bientot-app/src/workday.rs`, `flyout/body.rs`, `tray_model.rs`).
- [ ] T027 [US2] Onboarding « Jours ouvrés » row in the « Horaires » step (`crates/bientot-app/src/onboarding.rs`, `onboarding_view.rs`).
- [ ] T028 [US2] FR/EN strings; screenshot scenes (settings, part-time week, Saturday week, onboarding); GPUI design system and mockups artifacts updated; design review panel on the real screens; « En bref » lines.

**Checkpoint**: US2 acceptance scenarios pass; SC-002 and SC-005 property tests pass.

## Phase 4: User Story 3 — declare leave in the current week (P2)

**PR 4** `feat(app): declare a day or a half day of leave`.

- [ ] T030 [US3] Core command to set or clear a day's plan with undo (`crates/bientot-core/src/plans.rs`), refusing dates outside the current week; triggers a refresh.
- [ ] T031 [US3] Day-type selector in the open row (`crates/bientot-app/src/week.rs`, `week_view/details.rs`): « Journée travaillée / Demi-journée de congé / Journée de congé » with its note, fixed min-width, popup 5 px under its trigger; collapsed labels « Demi-journée » / « Congé ».
- [ ] T032 [US3] Week subtitle with counts and plurals; « Marquer comme congé » on a finished day without counted time (US1-3); uncounted-time lines « ne sont pas comptées » / « ne sont plus comptées » + « Annuler » (FR-024).
- [ ] T033 [US3] Flyout states « Journée de congé » and « Demi-journée de congé », « Temps travaillé — », « Passer en journée travaillée » (`crates/bientot-app/src/workday.rs`, `flyout/body.rs`).
- [ ] T034 [US3] Palette commands « Journée : marquer comme congé / demi-journée de congé / travaillée » (`crates/bientot-app/src/palette.rs`, tests in `palette/tests.rs`).
- [ ] T035 [US3] Export/import test: markings survive a `.bientot` round trip (`crates/bientot-store/src/transfer/tests.rs`).
- [ ] T036 [US3] FR/EN strings; screenshot scenes (selector, leave flyout, half-day flyout, « plus comptées » line); artifacts updated; design review panel; « En bref » lines.

**Checkpoint**: US3 acceptance scenarios pass.

## Phase 5: User Story 4 — public holidays (P2)

**PR 5** `feat(app): treat French public holidays as days off`.

- [ ] T040 [US4] Default `holiday_calendar` from the Windows region on first read (`crates/bientot-windows` region query behind a trait, `crates/bientot-core/src/settings.rs`); Linux tests with an injected region.
- [ ] T041 [US4] Resolver: holidays of the selected calendar on working days become `Holiday(id)` unless remembered as worked; « Passer en journée travaillée » and « Jour férié » update `worked_holidays` from the current week on; half day on a holiday does not (`crates/bientot-domain/src/schedule.rs`, `crates/bientot-core/src/plans.rs`).
- [ ] T042 [US4] Week row « Férié · <nom> », explanation block with its « aujourd'hui » variant and single button, selector with « Jour férié » (`crates/bientot-app/src/week*.rs`).
- [ ] T043 [US4] Holiday flyout (`crates/bientot-app/src/workday.rs`, `flyout/body.rs`).
- [ ] T044 [US4] Settings « Jours fériés » group under « Semaine type »: « Calendrier » select, « Jours fériés travaillés » list with « Retirer » (tooltip « Ce jour redevient férié. ») and empty state.
- [ ] T045 [US4] Palette « Journée : marquer comme jour férié » on built-in holidays only.
- [ ] T046 [US4] FR/EN strings (holiday names included); screenshot scenes (holiday week, holiday menu, holiday flyout, settings); artifacts updated; design review panel; « En bref » lines.

**Checkpoint**: US4 acceptance scenarios pass; `quickstart.md` passes end to end on Windows.

## Phase 6: Polish

- [ ] T050 [P] Update `docs/architecture.md` (schedule module, plan resolution) and `docs/privacy.md` if needed (no new data leaves the machine).
- [ ] T051 [P] Plugin snapshot: expose the day plan in `DaySnapshot` only if a later spec asks; note it in `specs/003-plugin-runtime` follow-ups instead of changing API 1.0 now.
- [ ] T052 Mark D-025 « implemented » with the release that ships the last PR.

## Dependencies & Execution Order

- Phase 1 (US1) is independent and ships first.
- Phase 2 blocks Phases 3–5.
- Phase 3 (US2) before Phases 4 and 5, because both rely on the seven-day calculation and the week view rows.
- Phases 4 and 5 are independent of each other; US1-3 (« Marquer comme congé ») lands with Phase 4.
- Within a phase, domain before core before app; tests with each task.

## Parallel opportunities

- T010 and T011; T012 and T013.
- In Phase 3, T023 and T024–T027 once T020–T022 are in.
- Phases 4 and 5 in parallel branches after Phase 3.
