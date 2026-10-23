# Tasks

## Lot 1. Explicit work during suspension, issue 65

- [x] T001 [US1] Add deterministic partial/full suspension, interval-add, edit and long-lock tests in tests/WorkTime.Domain.Tests/Adjustments/AdjustmentTests.cs.
- [x] T002 [US1] Update precedence in src/WorkTime.Domain/Adjustments/AdjustmentService.cs and remove obsolete rejection text/callers.
- [x] T003 [US1] Verify persisted correction and weekly totals in tests/WorkTime.App.Tests/History/SuspensionCorrectionIntegrationTests.cs.
- [x] T004 [US1] Reconcile FR-029 and product decisions in specs/001-core-workday-indicator/spec.md and docs/product-decisions.md.

## Lot 2. Hour and minute targets, issue 70

- [x] T005 [US2] Add duration parser/formatter and boundaries in src/WorkTime.App/Services/SettingsNumberParser.cs and tests/WorkTime.App.Tests/Settings/SettingsWorkspaceTests.cs.
- [x] T006 [US2] Replace target inputs and preserve grouped save behavior in src/WorkTime.App/Views/Settings/SettingsWindow.xaml and code-behind.
- [x] T007 [US2] Supply bilingual guidance and verify settings persistence.

## Lot 3. Guided corrections, issue 45

- [x] T008 [US3] Add raw interval correction requests, safe history queue handling and precise prefills in existing Presentation/History, Presentation/Adjustments and Composition code.
- [x] T009 [US3] Add source warning navigation and per-row correction actions in MainWindow and Views/History.
- [x] T010 [US3] Add exact interval, stale row, cancellation, restart and bilingual behavior checks.
- [x] T011 [US3] Add deterministic multiple-gap/suspended demo evidence in Services/Demo and exercise native correction.

## Lot 4. Qualitative lunch guidance, issue 53

- [x] T012 [US4] Add guidance and confidence states in Presentation/Workday with boundary tests.
- [x] T013 [US4] Add readable bilingual UI copy, tracking precedence and deterministic demo scenarios.

## Delivery

- [x] T014 Run full Release checks and native acceptance; record docs/daily-workflow-validation.md.
- [x] T015 Review code and decision trail independently and create PR #71.

The final independent shipping verdict, CI, review resolution and merge state
are tracked on PR #71. Delivery is
complete only when GitHub reports it merged and the merge is present in main.
