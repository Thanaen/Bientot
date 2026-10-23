# Tasks: Core Workday Indicator

**Input**: Design documents from `/specs/001-core-workday-indicator/`  
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/,
quickstart.md

**Tests**: Included because the constitution requires deterministic calculation
tests, ActivityWatch contract fixtures, Windows integration validation, and
release smoke coverage.

**Organization**: Tasks are grouped by user story so each phase produces an
independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel because it changes separate files and has no
  unmet dependency within the phase.
- **[Story]**: Maps directly to a user story in `spec.md`.

## Phase 1: Setup

**Purpose**: Create the pinned .NET/WinUI solution and continuous integration
baseline.

- [x] T001 Create `global.json`, `Directory.Build.props`, and `WorkTime.slnx` with the .NET 10/C# 14 Windows 11 x64 baseline
- [x] T002 Create centrally pinned stable dependencies in `Directory.Packages.props`
- [x] T003 [P] Create production projects under `src/WorkTime.App/`, `src/WorkTime.Domain/`, `src/WorkTime.ActivityWatch/`, `src/WorkTime.Persistence/`, and `src/WorkTime.Windows/`
- [x] T004 [P] Create xUnit projects under `tests/WorkTime.Domain.Tests/`, `tests/WorkTime.ActivityWatch.Tests/`, `tests/WorkTime.Persistence.Tests/`, `tests/WorkTime.Windows.Tests/`, and `tests/WorkTime.App.Tests/`
- [x] T005 [P] Create French and English resource skeletons in `src/WorkTime.App/Strings/fr-FR/Resources.resw` and `src/WorkTime.App/Strings/en-US/Resources.resw`
- [x] T006 Implement and document the WinUI 3 tray lifecycle spike in `spikes/TrayIconSpike/README.md` and `spikes/TrayIconSpike/`
- [x] T007 [P] Add restore, build, and test CI for Windows x64 in `.github/workflows/ci.yml`
- [x] T008 [P] Create the initial MSIX manifest and packaging project in `packaging/WorkTime.Package/`

---

## Phase 2: Foundational

**Purpose**: Shared domain, persistence, orchestration, diagnostics, and test
infrastructure required by every user story.

**Critical**: No user-story implementation begins until this phase is green.

- [x] T009 [P] Implement injected clock, timezone, local-workday, and UTC instant abstractions in `src/WorkTime.Domain/Time/`
- [x] T010 [P] Implement normalized interval and interval-set primitives in `src/WorkTime.Domain/Intervals/`
- [x] T011 [P] Implement ratified configuration models and defaults in `src/WorkTime.Domain/Configuration/WorkTimeSettings.cs`
- [x] T012 [P] Implement source freshness, quality, and degraded-state models in `src/WorkTime.Domain/Sources/`
- [x] T013 Create SQLite connection, forward-only migration runner, and schema v1 in `src/WorkTime.Persistence/Database/`
- [x] T014 [P] Implement JSON settings persistence in `src/WorkTime.Persistence/Settings/JsonSettingsStore.cs`
- [x] T015 [P] Implement current-user DPAPI secret storage in `src/WorkTime.Persistence/Secrets/DpapiSecretStore.cs`
- [x] T016 [P] Define tray, notification, startup, session, power, source, and persistence ports in `src/WorkTime.App/Abstractions/`
- [x] T017 Implement cancellable refresh orchestration and immutable summary publication in `src/WorkTime.App/Services/WorkdayRefreshCoordinator.cs`
- [x] T018 Implement bounded redacted local logging in `src/WorkTime.App/Diagnostics/`
- [x] T019 [P] Create frozen clock, timeline builder, and workday fixture helpers in `tests/WorkTime.Domain.Tests/Fixtures/`
- [x] T020 [P] Create sanitized ActivityWatch fixture infrastructure in `tests/WorkTime.ActivityWatch.Tests/Fixtures/`
- [x] T021 [P] Add timezone, DST, midnight, and clock-change tests in `tests/WorkTime.Domain.Tests/Time/WorkdayTimeTests.cs`
- [x] T022 Add transactional migration and rollback tests in `tests/WorkTime.Persistence.Tests/Database/MigrationTests.cs`

**Checkpoint**: Solution builds, foundational tests pass, and tray spike has a
go/no-go verdict.

---

## Phase 3: User Story 1 — Understand today's trajectory (P1) 🎯 MVP

**Goal**: Show a colour and accessible textual state from a deterministic daily
summary in the Windows tray.

**Independent Test**: Feed fixed timelines at exact green/orange/red/blue
boundaries and verify tray state and accessible text without ActivityWatch.

- [x] T023 [P] [US1] Write exact indicator threshold and blue-precedence tests in `tests/WorkTime.Domain.Tests/Forecasting/IndicatorStateTests.cs`
- [x] T024 [P] [US1] Write colour-independent presentation tests in `tests/WorkTime.App.Tests/Tray/TrayPresentationTests.cs`
- [x] T025 [US1] Implement `DailySummary` and indicator state value objects in `src/WorkTime.Domain/Summaries/`
- [x] T026 [US1] Implement indicator threshold evaluation in `src/WorkTime.Domain/Forecasting/IndicatorStateCalculator.cs`
- [x] T027 [P] [US1] Implement the tray abstraction with `H.NotifyIcon.WinUI` in `src/WorkTime.Windows/Tray/`
- [x] T028 [P] [US1] Create accessible coloured/degraded tray assets in `src/WorkTime.App/Assets/Tray/`
- [x] T029 [US1] Bind immutable summary state to tray icon and tooltip in `src/WorkTime.App/Presentation/Tray/TrayViewModel.cs`

**Checkpoint**: A deterministic demo changes tray state correctly and never
depends on colour alone.

---

## Phase 4: User Story 2 — See a useful forecast and overtime (P1)

**Goal**: Show post-lunch projected departure and credited overtime.

**Independent Test**: Evaluate frozen timelines before, during, and after lunch
and after completion, including exact visibility and overtime values.

- [x] T030 [P] [US2] Write lunch reservation, projection, and display-point tests in `tests/WorkTime.Domain.Tests/Forecasting/DepartureProjectionTests.cs`
- [x] T031 [P] [US2] Write credited overtime progression tests in `tests/WorkTime.Domain.Tests/Forecasting/OvertimeTests.cs`
- [x] T032 [US2] Implement the ratified departure projection model in `src/WorkTime.Domain/Forecasting/DepartureProjector.cs`
- [x] T033 [US2] Implement credited overtime calculation in `src/WorkTime.Domain/Forecasting/OvertimeCalculator.cs`
- [x] T034 [P] [US2] Implement compact flyout view and localization bindings in `src/WorkTime.App/Views/WorkdayFlyout.xaml`
- [x] T035 [US2] Implement flyout/tooltip visibility and value presentation in `src/WorkTime.App/Presentation/Workday/WorkdayFlyoutViewModel.cs`

**Checkpoint**: The compact view gives the correct projection after lunch and
switches to credited overtime when blue.

---

## Phase 5: User Story 3 — Account for activity and locked sessions (P1)

**Goal**: Produce explainable credited/excluded/provisional intervals from both
ActivityWatch server implementations and Windows state.

**Independent Test**: Run the accounting matrix against overlapping AFK,
session, lunch, suspend, and long-lock timelines and smoke both servers.

- [x] T036 [P] [US3] Add `aw-server-rust` info/bucket/event contract fixtures in `tests/WorkTime.ActivityWatch.Tests/Fixtures/Rust/`
- [x] T037 [P] [US3] Add Python `aw-server` info/bucket/event contract fixtures in `tests/WorkTime.ActivityWatch.Tests/Fixtures/Python/`
- [x] T038 [P] [US3] Write capability, authentication, bucket-selection, and malformed-event tests in `tests/WorkTime.ActivityWatch.Tests/ActivityWatchClientContractTests.cs`
- [x] T039 [P] [US3] Write overlap, lunch split, AFK, suspend, and provisional-lock tests in `tests/WorkTime.Domain.Tests/Accounting/AccountingEngineTests.cs`
- [x] T040 [US3] Implement typed read-only REST client and bearer handling in `src/WorkTime.ActivityWatch/ActivityWatchClient.cs`
- [x] T041 [US3] Implement AFK bucket discovery and user-visible ambiguity results in `src/WorkTime.ActivityWatch/AfkBucketSelector.cs`
- [x] T042 [US3] Implement defensive ActivityWatch event normalization in `src/WorkTime.ActivityWatch/AfkEventNormalizer.cs`
- [x] T043 [P] [US3] Implement Windows lock/unlock notifications and startup state probe in `src/WorkTime.Windows/Sessions/`
- [x] T044 [P] [US3] Implement Windows suspend/resume notifications in `src/WorkTime.Windows/Power/`
- [x] T045 [US3] Implement session event persistence in `src/WorkTime.Persistence/Sessions/SessionEventRepository.cs`
- [x] T046 [US3] Implement non-overlapping accounting and default classification matrix in `src/WorkTime.Domain/Accounting/AccountingEngine.cs`
- [x] T047 [US3] Add live smoke harnesses for both supported servers in `tests/WorkTime.ActivityWatch.Tests/Live/`

**Checkpoint**: Source integration is read-only, both servers satisfy the
contract, and the daily breakdown is explainable.

---

## Phase 6: User Story 5 — Control and correct the day (P1)

**Goal**: Pause/resume, finish a day, review long locks, and add or reclassify
time without rewriting ActivityWatch.

**Independent Test**: Apply every control to a fixed timeline, restart, and
verify persisted adjustments and zero double counting.

- [x] T048 [P] [US5] Write adjustment overlap, supersession, and idempotency tests in `tests/WorkTime.Domain.Tests/Adjustments/AdjustmentTests.cs`
- [x] T049 [P] [US5] Write pause/resume/finish lifecycle tests in `tests/WorkTime.Domain.Tests/Accounting/TrackingControlTests.cs`
- [x] T050 [P] [US5] Write 90-minute long-lock review tests in `tests/WorkTime.Domain.Tests/Accounting/LongLockReviewTests.cs`
- [x] T051 [US5] Implement manual adjustment persistence in `src/WorkTime.Persistence/Adjustments/ManualAdjustmentRepository.cs`
- [x] T052 [US5] Implement pause, resume, and finish-day commands in `src/WorkTime.App/Services/TrackingControlService.cs`
- [x] T053 [US5] Implement adjustment validation and interval de-overlap in `src/WorkTime.Domain/Adjustments/AdjustmentService.cs`
- [x] T054 [US5] Implement provisional long-lock review workflow in `src/WorkTime.App/Services/LongLockReviewService.cs`
- [x] T055 [P] [US5] Implement add/edit/reclassify dialog in `src/WorkTime.App/Views/AdjustTimeDialog.xaml`
- [x] T056 [P] [US5] Add pause/resume/finish/add-time tray commands in `src/WorkTime.App/Presentation/Tray/TrayCommandViewModel.cs`
- [x] T057 [US5] Add restart persistence and double-count integration tests in `tests/WorkTime.App.Tests/Controls/TrackingControlIntegrationTests.cs`

**Checkpoint**: Users can correct source limitations locally and all corrections
remain visible and reversible.

---

## Phase 7: User Story 4 — Configure personal rules (P2)

**Goal**: Edit all ratified defaults safely and recalculate immediately.

**Independent Test**: Change each setting, reject invalid combinations, restart,
and verify persistence and live recalculation.

- [x] T058 [P] [US4] Write settings default and cross-field validation tests in `tests/WorkTime.Domain.Tests/Configuration/WorkTimeSettingsTests.cs`
- [x] T059 [P] [US4] Write JSON migration and invalid-file recovery tests in `tests/WorkTime.Persistence.Tests/Settings/JsonSettingsStoreTests.cs`
- [x] T060 [US4] Implement settings validation in `src/WorkTime.Domain/Configuration/WorkTimeSettingsValidator.cs`
- [x] T061 [US4] Implement settings orchestration and immediate refresh in `src/WorkTime.App/Services/SettingsService.cs`
- [x] T062 [P] [US4] Implement General and Schedule settings pages in `src/WorkTime.App/Views/Settings/`
- [x] T063 [P] [US4] Implement Accounting and Notifications settings pages in `src/WorkTime.App/Views/Settings/`
- [x] T064 [US4] Add restart and live-recalculation UI tests in `tests/WorkTime.App.Tests/Settings/SettingsIntegrationTests.cs`

2026-09-14: The adaptive native settings window completes all four categories.
Service tests and actual WinUI journeys verify restart, live recalculation,
invalid drafts, failed writes and retry. See
settings validation.

**Checkpoint**: Every product default is editable without direct file changes.

---

## Phase 8: User Story 6 — Use surplus on another workday (P2)

**Goal**: Earn, apply, recompute, and expire positive weekly credit with a
configurable 6-hour default floor.

**Independent Test**: Run Monday–Friday scenarios including retroactive edits,
floor-limited consumption, and Friday expiry.

- [x] T065 [P] [US6] Write weekly earning, floor, consumption, and expiry tests in `tests/WorkTime.Domain.Tests/Credit/WeeklyCreditTests.cs`
- [x] T066 [P] [US6] Write retroactive correction cascade tests in `tests/WorkTime.Domain.Tests/Credit/CreditRecalculationTests.cs`
- [x] T067 [US6] Implement weekly credit policy in `src/WorkTime.Domain/Credit/WeeklyCreditPolicy.cs`

> Scope: T065-T067 provide only the pure weekly domain calculation. Persistence, orchestration, summaries, and presentation remain in T068-T072.
- [x] T068 [US6] Implement credit ledger persistence in `src/WorkTime.Persistence/Credit/CreditLedgerRepository.cs`

> T068 scope: persist exact caller-supplied weekly ledger movements and provenance only. Runtime recalculation, summary integration, and presentation remain in T069-T072.
- [x] T069 [US6] Implement deterministic earliest-change-through-Friday recalculation in `src/WorkTime.App/Services/WeeklyRecalculationService.cs`

  2026-09-09: The live app now recalculates all observed weekdays from one
  captured context and commits their summaries and credit ledger before
  publication. Historical corrections, restart outages, stale retained source
  evidence, revision conflicts, rollback, and Friday expiry are covered.
  2026-09-14: Later refreshes now discover prior weeks whose latest observed
  head retains credit. WorkTime replays those weeks from Monday through Friday
  with fresh source evidence, preserves their persisted timezone, and commits
  expiry before it publishes the current week. Missing evidence defers only the
  affected old week. Current-week reconstruction does not fabricate future
  weekdays.

- [x] T070 [US6] Integrate effective target and new overtime into summaries in `src/WorkTime.Domain/Summaries/DailySummaryBuilder.cs`
- [x] T071 [P] [US6] Add worked/applied/available credit presentation in `src/WorkTime.App/Presentation/Workday/CreditViewModel.cs`
- [x] T072 [US6] Add multi-day persistence integration tests in `tests/WorkTime.App.Tests/Services/WeeklyRecalculationServiceTests.cs`

  Production ordinary and immediate refreshes use the weekly service. Demo
  scenarios retain nominal calculations so synthetic clock changes remain
  possible. The Credit demo uses real daily accounting and the pure weekly
  policy. T071 displays worked, applied, and available credit with an explicit
  target breakdown; French and English native acceptance passed on 2026-09-10.

**Checkpoint**: The Monday +30 / Tuesday 6h30 example and every expiry/floor case
match the ratified model.

---

## Phase 9: User Story 7 — Receive useful approach warnings (P2)

**Goal**: Deliver deduplicated completion and latest-departure Windows warnings.

**Independent Test**: Cross both thresholds with a frozen clock, separately,
together, after restart, and in reverse chronology.

- [x] T073 [P] [US7] Write completion/latest/merge threshold tests in `tests/WorkTime.Domain.Tests/Notifications/NotificationEligibilityTests.cs`
- [x] T074 [P] [US7] Write restart deduplication tests in `tests/WorkTime.Persistence.Tests/Notifications/NotificationReceiptTests.cs`
- [x] T075 [US7] Implement notification eligibility and merge rules in `src/WorkTime.Domain/Notifications/NotificationPolicy.cs`
- [x] T076 [US7] Implement receipt persistence with unique trigger keys in `src/WorkTime.Persistence/Notifications/NotificationReceiptRepository.cs`
- [x] T077 [P] [US7] Implement Windows App SDK notification adapter in `src/WorkTime.Windows/Notifications/WindowsNotificationService.cs`
- [x] T078 [US7] Implement notification scheduling after summary publication in `src/WorkTime.App/Services/NotificationCoordinator.cs`
- [x] T079 [P] [US7] Add localized notification templates in `src/WorkTime.App/Strings/fr-FR/Resources.resw` and `src/WorkTime.App/Strings/en-US/Resources.resw`
- [x] T080 [US7] Add same-minute merge and post-limit completion integration tests in `tests/WorkTime.App.Tests/Notifications/NotificationIntegrationTests.cs`

2026-09-06: T073-T080 implemented with editable notification preferences in the schedule dialog. T063 remains open for Accounting settings. See scope and validation.

**Checkpoint**: Each trigger appears at most once per day and carries useful
context.

---

## Phase 10: User Story 8 — Diagnose source problems (P2)

**Goal**: Surface missing, stale, ambiguous, authenticated, malformed, and
recovering ActivityWatch states without misleading totals.

**Independent Test**: Simulate every source health state and recovery while
preserving the last healthy value as explicitly stale.

- [x] T081 [P] [US8] Verify source health invariants in `tests/WorkTime.Domain.Tests/Sources/SourceHealthTests.cs` and quality transitions in `tests/WorkTime.App.Tests/Services/SourceRecoveryIntegrationTests.cs`
- [x] T082 [P] [US8] Cover timeout, 401, empty-bucket, malformed, and recovery in `tests/WorkTime.ActivityWatch.Tests/ActivityWatchClientContractTests.cs`, with retained stale evidence in `tests/WorkTime.App.Tests/Services/ActivityWatchPollingServiceTests.cs`
- [x] T083 [US8] Implement 30-second polling and capped failure backoff in `src/WorkTime.App/Services/ActivityWatchPollingService.cs`
- [x] T084 [US8] Implement unknown-gap construction for incomplete history in `src/WorkTime.Domain/Sources/UnknownGapBuilder.cs`
- [x] T085 [US8] Implement source diagnostics and accounting-confidence models in `src/WorkTime.Domain/Sources/SourceDiagnostics.cs`, `src/WorkTime.Domain/Forecasting/AccountingConfidence.cs`, and `src/WorkTime.App/Presentation/Workday/WorkdayFlyoutViewModel.cs`
- [x] T086 [P] [US8] Implement source-warning tray state and localized recovery diagnostics in `src/WorkTime.App/Presentation/Tray/TrayViewModel.cs` and `src/WorkTime.App/Views/WorkdayFlyout.xaml`
- [x] T087 [US8] Add outage/recovery and late-start integration tests in `tests/WorkTime.App.Tests/Services/SourceRecoveryIntegrationTests.cs`

2026-09-06: Source retention and retry admission sit behind the existing source
port. Local refresh still runs every 30 seconds. Diagnostics and recovery views
are completed in T085-T086. See the source reliability slice
for behavior, limits, and verification.

**Checkpoint**: WorkTime never presents an unavailable source as a valid zero.

---

## Phase 11: User Story 9 — Review the current week (P2)

**Goal**: Show detailed today accounting and editable Monday–Friday summaries.

**Independent Test**: Populate, navigate, edit, restart, and verify recalculated
history for five workdays.

- [x] T088 [P] [US9] Write daily summary revision repository tests in `tests/WorkTime.Persistence.Tests/History/DailySummaryRepositoryTests.cs`
- [x] T089 [P] [US9] Write current-week query and ordering tests in `tests/WorkTime.Domain.Tests/History/CurrentWeekTests.cs`
- [x] T090 [US9] Implement daily summary revision persistence in `src/WorkTime.Persistence/History/DailySummaryRepository.cs`

  2026-09-08: Added immutable typed snapshots, contiguous idempotent append,
  exact tick persistence, latest-day/week reads, and transactional schema-v7
  migration with legacy backup/recovery coverage. Fingerprint generation,
  write cadence, atomic summary-plus-credit publication, and history UI remain
  in T069 and T091-T094.
- [x] T091 [US9] Implement current-week history query in `src/WorkTime.App/Services/HistoryService.cs`
- [x] T092 [P] [US9] Implement the detailed day view in the adaptive native `src/WorkTime.App/Views/History/HistoryWindow.xaml`
- [x] T093 [P] [US9] Implement the Monday–Friday summary in the adaptive native `src/WorkTime.App/Views/History/HistoryWindow.xaml`
- [x] T094 [US9] Add edit/recalculate/restart integration tests in `tests/WorkTime.App.Tests/History/HistoryIntegrationTests.cs`

  2026-09-14: Service integration now saves a Monday correction on Friday,
  recalculates all five daily heads and the credit ledger, and reads the same
  corrected week after reopening SQLite. Target-day overlap, timezone, and
  future-date rejection are covered. T092 and T093 now share one native
  list/detail window. Native FR/EN and light/dark journeys, compact keyboard
  navigation, short-window correction dialogs, and persisted correction totals
  passed; see `docs/credit-history-validation.md`.

**Checkpoint**: The current week is understandable and correctable without
ActivityWatch's dashboard.

---

## Phase 12: User Story 10 — Set up and maintain WorkTime safely (P2)

**Goal**: Complete bilingual onboarding, secure ActivityWatch connection,
autostart, updates, diagnostics, and public packaging.

**Independent Test**: Complete first-launch and update flows on a clean Windows
11 x64 VM against both servers, missing server, authentication, and both
languages.

- [x] T095 [P] [US10] Write onboarding state-machine tests in `tests/WorkTime.App.Tests/Onboarding/OnboardingTests.cs`

  2026-10-07: A pure reducer in `src/WorkTime.App/Services/Onboarding/` drives
  language, ActivityWatch endpoint/API key/health, schedule, start-with-Windows
  consent, and confirmation. `OnboardingCompletion` applies the confirmed
  startup choice, stores the API key as a secret, and saves settings only after
  confirmation. 43 cases pass. Bucket selection during onboarding is left to
  T098; the onboarding pages themselves remain T098/T099.

- [x] T096 [P] [US10] Write DPAPI secret round-trip and redaction tests in `tests/WorkTime.Persistence.Tests/Secrets/DpapiSecretStoreTests.cs`

  2026-09-08: Nine real Windows DPAPI cases cover store reopening, Unicode,
  replacement, deletion, cancellation, corrupted ciphertext, and identifier
  isolation. Eight additional cases in
  `tests/WorkTime.App.Tests/Diagnostics/LocalDiagnosticLoggerTests.cs` cover
  credential redaction. Both affected projects pass with 307 tests total.
  Process restarts and access from another Windows account were not exercised.

- [x] T097 [P] [US10] Write GitHub Releases metadata and daily-throttle tests in `tests/WorkTime.App.Tests/Updates/UpdateCheckTests.cs`

  2026-10-08: `UpdateCheck` in `src/WorkTime.App/Services/Updates/` decides
  whether a check is due (enabled and never checked, a day elapsed, or a last
  check in the future after a clock change) and turns a `releases/latest` body
  into up to date, update available, or unusable metadata. Only a strictly
  newer, non-draft, non-prerelease release with an https github.com page is
  offered; nothing is installed. 56 cases pass. The HTTP client, recording the
  check time, and the 404 returned while `alpha-latest` is the only
  (prerelease) release are left to T104; the offer dialog to T105.

- [x] T098 [US10] Implement ActivityWatch endpoint/API-key onboarding in `src/WorkTime.App/Views/Onboarding/ActivityWatchStep.xaml`

  2026-10-09: `ActivityWatchStep` is a `UserControl` that edits the endpoint
  and optional API key, tests the connection, shows the status in a polite
  InfoBar, links to the official install page when ActivityWatch or its AFK
  tracking is missing, and offers ambiguous AFK buckets. It raises
  `OnboardingCommand` values and never displays the key. The reducer gained
  `SelectBucket` and `BucketCandidates`; a check result is ignored when the
  endpoint, key, or bucket changed after it started.
  `ActivityWatchConnectionProbe` reads one settlement horizon with the backoff
  bypassed, and `ActivityWatchStepPresentation` maps each status to its
  resources, severity, and install link. English and French strings ship
  together. 59 onboarding cases pass (48 reducer, 3 probe, 8 presentation and
  view). The onboarding host window and wiring the probe's source factory to
  `ActivityWatchClient` with the DPAPI flow are left to T099. Only the Windows
  CI build compiles the XAML, and the step has not run on Windows.

- [x] T099 [US10] Implement language, schedule, and start-with-Windows onboarding in `src/WorkTime.App/Views/Onboarding/`

  2026-10-11: `OnboardingWindow` hosts four steps, following the Onboarding
  boards: language (Windows, French, or English), the typical day (daily
  target, lunch, latest departure), start with Windows (on by default, or the
  state Windows or a policy imposes), then the existing ActivityWatch step,
  whose Finish is the explicit confirmation. The reducer now orders the steps
  that way and `ChooseLanguage(null)` follows Windows again.
  `OnboardingPresentation` describes each page, reads the schedule (the
  minimum target is kept unless the daily target is shorter), and the startup
  choice; it reuses `SettingsNumberParser`, `StartupSettingsService`, and
  `OnboardingCompletion`. The onboarding opens when no settings file exists
  and runs before anything else starts; closing it saves nothing and quits.
  The probe builds an `ActivityWatchClient` with the entered key, which
  completion stores with DPAPI for the main client. A new language restarts
  WorkTime through `AppInstance.Restart`. Demo runs skip the onboarding unless
  `--onboarding` asks for it; the screenshot script captures its four steps,
  and the portable smoke test launches both a first run and a configured run.
  98 onboarding cases pass on Linux, 38 of them new; the reordered reducer
  cases were updated.
  Only the Windows CI build compiles the XAML; the window, the restart, and
  the DPAPI hand-off have not run on Windows.
- [x] T100 [US10] Implement packaged and portable startup registration adapter in `src/WorkTime.Windows/Startup/WindowsStartupService.cs`
- [x] T101 [P] [US10] Complete French resources in `src/WorkTime.App/Strings/fr-FR/Resources.resw`

  2026-10-11: T103 found two French gaps, both shared with English: the
  main window's title bar and heading showed a hard-coded "WorkTime". They now
  read `ShellWindowTitle` and `ShellTitle.Text`. French punctuation now keeps
  its line together: a no-break space before `:` and inside « », a narrow
  no-break space before `;` and `?`. The credit breakdown subtracts with a
  minus sign (−) instead of a hyphen. The tray tooltip cases expect the
  no-break space. All 488 French strings were read again; no other gap was
  found. Spaces between numbers and units ("27 h 56 min") are unchanged.
  The no-break spaces have not been seen rendered on Windows (window, tray
  tooltip, toast), and only the Windows CI build compiles the changed
  `MainWindow`.

- [x] T102 [P] [US10] Complete English resources in `src/WorkTime.App/Strings/en-US/Resources.resw`

  2026-10-11: English gained the same two title keys. "Today's" and
  "week's" now use the typographic apostrophe, like the other English
  strings, and the credit breakdown uses a minus sign. No other gap was
  found. Not checked on Windows.

- [x] T103 [US10] Add hard-coded/missing resource validation in `tests/WorkTime.App.Tests/Localization/ResourceCompletenessTests.cs`

  2026-10-11: `ResourceCompletenessTests` (18 cases, under a second on
  Linux) reads the `.resw`, XAML, and C# sources. It fails when a key is
  defined twice, exists in only one language, or has an empty value; when
  format placeholders differ between languages; when an `x:Uid` has no
  `<Uid>.<Property>` resource; when a literal key read by
  `GetString`/`getString`/`getText`, or a PascalCase literal that starts like
  an existing key (status and error keys handed to a lookup later), is
  missing; when an interpolated key such as `$"Settings{tag}Title"` matches
  no key; when an enum value behind an interpolated key (indicator,
  disposition, label, source, timeline kind, demo scenario) or a settings
  navigation tag has no string; when XAML sets visible text (`Text`,
  `Content`, `Header`, `PlaceholderText`, `Title`, `Message`, `Description`,
  dialog button texts, `OnContent`/`OffContent`, tooltip, automation name or
  help text) to a literal containing a letter, or holds text as element
  content; when C# assigns such a literal to one of those properties or
  passes one to `SetName`/`SetHelpText`/`SetToolTip`; when a French string
  has a breaking space before `: ; ! ? »` or after «; and when an English
  string has a straight apostrophe. A literal without any letter (·, 40, +)
  is allowed; there is no word allowlist. Removing `OnboardingStepLanguage`
  from French, `ShellLoading` or `StartupDisabledByPolicy` from both, every
  `ShellRefresh.*` entry, `HistoryLabelAfk`, or the settings page titles,
  emptying a value, or changing a placeholder each made the matching case
  fail; the original MainWindow literals failed the two hard-coded checks.
  Keys built in code from a variable without a literal (for example
  `GetString(key)` with `key` computed elsewhere) are covered only when the
  literal appears somewhere as a key-like string; text concatenated after a
  variable in C# is not detected. Unused resources are not reported.

- [x] T104 [US10] Implement the once-daily Microsoft Store update check for the packaged build in `src/WorkTime.App/Services/Updates/StoreUpdateService.cs`

  2026-10-11: Revised for the Store decision (D-016, #108). The GitHub Releases
  client is dropped. `StoreUpdateService` runs only with package identity,
  reuses `UpdateCheck.IsDue` for the daily throttle and records the check time,
  then calls `StoreContext.GetDefault()`, gives the context the main window's
  HWND with `WinRT.Interop.InitializeWithWindow.Initialize`, and calls
  `GetAppAndOptionalStorePackageUpdatesAsync()` on the UI thread (the API
  returns `ERROR_INVALID_WINDOW_HANDLE` otherwise). A non-empty list becomes an
  update-available outcome; an error is logged and treated as no offer. The
  portable build skips the check. `UpdateCheck.Evaluate`, `GitHubRelease`, and
  the GitHub metadata cases from T097 are removed, since the Store returns the
  updates itself; the throttle cases stay.

  2026-10-12: `StoreUpdateService` takes an `IStoreUpdatePort`, which returns
  an `IStoreUpdateOffer` or null; only an offer can be installed. A null port
  stands for the portable build, which never checks and never records a check.
  When `UpdateCheck.IsDue` allows it, the service records `LastCheckedAtUtc`
  through `SettingsService`, then asks the Store. The time is recorded first,
  so a failing or hanging Store is still asked only once a day. A Store error
  is logged as `update_check.failed` with its HRESULT and offers nothing.
  `StoreContextUpdatePort` in `WorkTime.Windows` returns null without package
  identity. Otherwise it initializes one `StoreContext` with the main window
  handle and runs both Store calls on that window's dispatcher.
  `PackageIdentity.IsPresent` is now shared with the startup adapter.
  `App` checks once initialization ends, then every hour with a
  `PeriodicRefreshService`. `GitHubRelease`, `UpdateCheck.Evaluate`,
  `UpdateCheckOutcome`, and their 47 cases are gone; the 9 throttle cases stay.
  10 new service cases run on Linux against a real `JsonSettingsStore` and a
  fake port. They cover an update found, none found, a Store error, a second
  check on the same day, 23 hours and one day after a check, checks turned
  off, the portable build, a cancelled check, and settings that cannot be
  saved. Moving the recording after the Store call made two cases fail. The
  adapter compiles on Linux (`-p:AppxGeneratePriEnabled=false`). The real
  Store calls have not run: they need WorkTime published in the Store (T107).

- [x] T105 [US10] Implement explicit Store update offer UI in `src/WorkTime.App/Views/Updates/UpdateAvailableDialog.xaml`

  2026-10-11: Revised for the Store decision (D-016, #108). The dialog offers
  the update found by T104. Only the user's "Update" action calls
  `StoreContext.RequestDownloadAndInstallStorePackageUpdatesAsync`, on the
  context T104 already initialized with the owner window; Windows then asks
  for permission before downloading, and the
  install may close WorkTime. The `TrySilentDownload*` methods are not used.
  Declining keeps WorkTime running and offers again after the next daily check.

  2026-10-12: An available update first shows as a closable `UpdateNotice`
  InfoBar in the main window. Checks also run while the window is hidden in
  the tray, and a ContentDialog cannot open over another one such as Add time,
  so the dialog only opens from the notice's "See the update" action.
  `UpdateAvailableDialog` has "Update" as its default and only accent button
  and "Later" as its close button, both 40 px high, in French and English.
  Only "Update" calls `IStoreUpdateOffer.DownloadAndInstallAsync`, which
  hands the updates found to `RequestDownloadAndInstallStorePackageUpdatesAsync`
  on the same `StoreContext`. "Later" or closing the notice drops the offer
  until the next daily check. A Store result other than completed or cancelled
  shows "The update could not be installed" and is logged. `--demo-update`
  makes a demo run offer a fake update that installs nothing. The screenshot
  script captures `update-notice.png` and `update-dialog.png` in each
  language and theme. 4 Linux cases check the dialog markup, the notice, and
  the French and English labels; `ResourceCompletenessTests` covers the new
  keys. Only the Windows CI build compiles the XAML. Neither the dialog, the
  screenshot steps, nor a real Store download and install has run on Windows.

- [x] T106 [US10] Implement redacted diagnostic ZIP export in `src/WorkTime.App/Diagnostics/DiagnosticExportService.cs`
- [ ] T107 [US10] Prepare Microsoft Store publication: Partner Center app reservation, package identity in `packaging/WorkTime.Package/Package.appxmanifest`, privacy policy URL, and an `msstore` submission job in `.github/workflows/release.yml` on `v*` tags behind a protected `release` environment, alongside the unsigned portable x64 ZIP

  2026-10-11: Revised for the Store decision (D-016, #108). No code-signing
  certificate and no `docs/code-signing.md`: the Store re-signs the MSIX after
  certification. The owner creates the Store developer account, reserves the
  name, and provides the privacy policy URL; the `Identity Name` and
  `Publisher` assigned by Partner Center replace the `Thanaen.WorkTime` and
  `CN=Thanaen` placeholders. The submission job needs approval in the
  `release` environment and keeps its Partner Center credentials as
  environment secrets. The `alpha-latest` ZIP release is unchanged.

- [ ] T108 [US10] Document clean-VM onboarding, update, and package smoke results in `docs/release-checklist.md`

**Checkpoint**: A public v1 candidate installs, onboards, runs in both
languages, updates only with consent, and exports no sensitive activity data.

---

## Phase 13: Polish & Cross-Cutting Concerns

**Purpose**: Enforce constitution quality gates and prepare the first release.

- [ ] T109 [P] Run and document keyboard, screen-reader, contrast, and non-colour state validation in `docs/accessibility-checklist.md`
- [x] T110 [P] Add Explorer restart, DPI, theme, and multi-monitor tray tests in `tests/WorkTime.Windows.Tests/Tray/TrayLifecycleTests.cs`

  2026-10-11: Two product fixes came with the tests. Explorer restart:
  H.NotifyIcon re-adds the icon on `TaskbarCreated` but swallows a failed
  add, so `NotifyIconTrayPort` now listens to the same broadcast and re-adds
  a shown icon that is still missing; a refused add waits for the next
  broadcast, and a hidden or disposed port does nothing. DPI: the tray ICOs
  held only 16, 32 and larger frames, so at 125 %, 150 % and 175 % the
  notification area rescaled a 16 or 32 px bitmap. Each ICO now also has
  20, 24, 28, 36 and 40 px frames rendered from the SVG; the existing frames
  are byte-identical. `TrayLifecycleTests` (23 cases) covers the restart
  handling with fakes (7 cases, which also pass on Linux) and, on Windows
  only, checks that `GetSystemMetricsForDpi(SM_CXSMICON)` is 16, 20, 24, 28,
  32, 36, 40 and 48 px at 96 to 288 dpi and that `System.Drawing.Icon`
  loads exactly that frame from every asset. Two Linux files sit outside the
  named path: `tests/WorkTime.App.Tests/Tray/TrayIconDisplayTests.cs` reads
  the ICO directories and SVG colours, and
  `tests/WorkTime.App.Tests/Tray/TrayWindowPlacementTests.cs` covers
  `TrayWindowPlacement`, now extracted unchanged from `MainWindow` (the
  window hosting the compact view). It checks centring in the work area of
  displays left of and above the primary (negative coordinates), clamping
  to a short work area, and 125 % and 150 % scales. There are no light and
  dark icon variants: each symbol is white on its own opaque disc
  (at least 4.5:1), every disc is at least 3:1 against the light taskbar,
  and the dark degraded badge has a white outline. Against the dark taskbar
  (#202020) the blue, orange and red discs measure about 2.4:1 to 2.7:1;
  this is reported, not asserted, and needs a design decision. The context
  menu is H.NotifyIcon's native popup at the cursor, so Windows positions it;
  H.NotifyIcon 2.4.1 decodes the icon's anchor point without its sign, which
  could misplace the menu if the notification area ever sits left of or
  above the primary display. Not run on real hardware: an actual Explorer
  restart, live scale changes (the icon size follows the system DPI and is
  not reloaded after a change), mixed-DPI or multi-monitor setups, and
  how the icons look on either taskbar theme. Only the Windows CI compiles
  the changed `MainWindow`.

- [x] T111 Add idle CPU, working-set, and startup performance harness in `tests/WorkTime.App.Tests/Performance/ReleasePerformanceTests.cs`

  2026-10-12: The harness is `scripts/measure-performance.ps1`, not the
  named C# file: the budgets are about the published WinUI executable,
  which the Linux test project cannot launch. It starts the portable
  publication as a sign-in launch (`--worktime-windows-startup`, an
  isolated `--data-dir` with saved settings, as the startup smoke test
  does). Startup is the time from process creation to the new
  `startup.tray_ready` diagnostic event, which `App` writes once the tray
  icon is shown and the first refresh has run. After a settle period it
  samples the process every second: average CPU is the processor-time
  delta over the wall time and the logical processor count, and it reports
  peak and last working set, lifetime peak working set and peak private
  bytes. The Windows CI job runs it on every build after the launch smoke
  tests, with a 30 s idle window after a 5 s settle; the window covers one
  30 s periodic refresh. It adds about 40 s (estimated, not yet measured on
  a runner; the job summary prints the harness time). CI writes the results
  to the job summary, warns above the spec budgets (0.5 % CPU, 150 MiB,
  3 s) and fails only above 2 % CPU, 200 MiB or 6 s, because a 30 s window
  on a shared runner without ActivityWatch is not the release VM. On the
  release VM, `-Release` runs the 15-minute idle window and fails at the
  spec budgets; T116 and T117 record its table. Still needed there: a run
  with ActivityWatch available (CI measures the source-unavailable state),
  the packaged MSIX rather than the portable build, and startup measured
  from sign-in rather than from process creation.
- [x] T112 [P] Add diagnostic-export secret/window/app-data leak tests in `tests/WorkTime.App.Tests/Diagnostics/DiagnosticPrivacyTests.cs`
- [x] T113 [P] Add package dependency and license inventory generation in `.github/workflows/dependency-review.yml`
- [x] T114 Configure branch build quality gates and release artifact retention in `.github/workflows/ci.yml` (workflow gates and retention are complete; repository-plan limits prevent required-check enforcement)
- [x] T115 [P] Update architecture, privacy, troubleshooting, and contributor guidance in `docs/`

  2026-10-12: Added `docs/architecture.md` (projects and references from
  `WorkTime.slnx`, data flow from ActivityWatch to tray, window and
  notifications, where state lives, ports and adapters, Store updates),
  `docs/privacy.md` (stored files, the two kinds of network request, what the
  diagnostic export keeps and drops, how to delete data),
  `docs/troubleshooting.md` (source, startup, tray, update and other notices,
  quoted from the English resources, and how to collect diagnostics) and
  `CONTRIBUTING.md` at the root (SDK, Linux and Windows tests, what only
  Windows CI checks, CI jobs and cost rules, localization, commit and spec
  conventions). Each claim was checked against the source files the pages
  name, and every relative link resolves. The README links the new pages and
  no longer says that custom endpoint and API-key onboarding are missing.
  Not covered: how packaged `AppData` virtualization affects data locations
  and deletion, which T117 validates on a Store install.
- [ ] T116 Execute and record every scenario from `specs/001-core-workday-indicator/quickstart.md` in `docs/validation-report.md`
- [ ] T117 Perform Microsoft Store install, update, uninstall, and startup validation in `docs/release-checklist.md`

  2026-10-11: Revised for the Store decision (D-016, #108). Validate the
  Store-installed package: install from the Store listing, update through both
  the Store and the in-app offer (T105), uninstall, and start with Windows
  through `WorkTimeStartupTask`. Also check that data left by a portable alpha
  in `%LocalAppData%\WorkTime` is found after installing the Store build,
  given packaged `AppData` write virtualization (see plan, "Packaging and
  updates").

- [x] T118 Update `README.md` with supported platform, ActivityWatch variants, install, privacy, and release links
- [x] T119 [P] Add FsCheck interval invariants for no double counting, split conservation, and covered-duration bounds in `tests/WorkTime.Domain.Tests/Intervals/IntervalPropertyTests.cs`

---

## Phase 14: Built-in activity source (D-021)

**Goal**: Count activity without ActivityWatch. A built-in source polls
`GetLastInputInfo` and is the default; ActivityWatch stays available as an
optional source chosen in settings, never combined with the built-in source
for the same period.

**Independent Test**: On a clean Windows 11 x64 VM without ActivityWatch,
complete onboarding, work, stay idle past the threshold, lock, suspend, and
restart WorkTime; verify the credited and excluded totals. Then select a
detected ActivityWatch in the middle of a workday and verify each part of the
day is read from one source only.

The specification, decisions, data model, and contracts were updated first
(2026-10-11, D-021, FR-044 to FR-046, and the
[built-in activity source contract](contracts/built-in-activity-source.md)).
The remaining work follows in three steps: the source and its storage
(T120-T125), the choice in settings (T126-T128), then onboarding without the
ActivityWatch prerequisite (T129-T131).

- [x] T120 [P] [US3] Write injected-clock tests for idle threshold crossing and exact boundary, backdated idle start, return to active, start while idle, threshold change, tick-count wraparound, suspend, lock, crash, and restart, the settled boundary of an `active` span (the last input a sample read, never a later sample or a stop instant), and recovery after missed samples in the same state (a new span, no bridging), in `tests/WorkTime.Domain.Tests/Sources/IdleStateTrackerTests.cs`; and, through the sampler and `ReadAsync`, a clean stop with a pending tail (last input 10:00, stop 10:01, threshold not elapsed) read again hours later and after a threshold change, and a crash with a pending tail followed by a restart, both proving that credited activity ends at the stored settled boundary and the rest stays unknown, in `tests/WorkTime.App.Tests/Services/BuiltInActivitySourceTests.cs`
- [x] T121 [P] [US3] Write `ActivitySpan` and `ActivitySourceSelection` repository tests (append, close on transition, `SettledThroughUtc` advance, idempotent replay, unknown gaps, a startup span appended after an `active` span with an earlier `SettledThroughUtc` then a reload that still shows the gap as unknown and the earlier span unchanged, selection lookup by instant, no source before the first selection, and the legacy epoch selection appended once) in `tests/WorkTime.Persistence.Tests/Activity/ActivityLedgerRepositoryTests.cs`
- [x] T122 [US3] Add the schema migration and repositories for `ActivitySpan` and `ActivitySourceSelection` in `src/WorkTime.Persistence/Activity/`
- [x] T123 [US3] Implement the pure idle state machine in `src/WorkTime.Domain/Sources/IdleStateTracker.cs`, a last-input port in `src/WorkTime.App/Abstractions/Sources/ILastInputPort.cs` with its `GetLastInputInfo`/`GetTickCount` adapter in `src/WorkTime.Windows/Activity/LastInputPort.cs`, and the 5-second sampler that records spans and implements `IActivitySourcePort` in `src/WorkTime.App/Services/BuiltInActivitySource.cs`
- [x] T124 [US3] Make the bucket identifier optional on `ActivityReadResult.Snapshot` and keep `SelectionRequired` specific to ActivityWatch in `src/WorkTime.App/Abstractions/Sources/IActivitySourcePort.cs`, migrating every caller in the same change
- [x] T125 [US3] Route each read to the source selected for each instant, splitting a request at every selection change, never merging the two sources, and returning unknown without reading any source before the first selection; at startup, append the legacy `activitywatch` selection at the epoch for a settings file without an activity source value, in `src/WorkTime.App/Services/ActivitySourceRouter.cs`. Tests in `tests/WorkTime.App.Tests/Services/ActivitySourceRouterTests.cs` cover a fresh built-in install, where a day and a week query crossing onboarding confirmation return unknown before confirmation and never call the ActivityWatch fake, and a legacy install, where the same queries read ActivityWatch before the upgrade and the seed is a no-op on a second start
- [x] T126 [P] [US4] Add the selected activity source (a missing value marks a settings file written before D-021 and reads as ActivityWatch) and the built-in idle threshold (default 180 s, 30 s to 30 min) with validation in `src/WorkTime.Domain/Configuration/WorkTimeSettings.cs` and `src/WorkTime.Domain/Configuration/WorkTimeSettingsValidator.cs`
- [x] T127 [US4] Add the “Built-in” and “ActivityWatch” choice and the idle threshold to the Activity source page in `src/WorkTime.App/Views/Settings/SettingsWindow.xaml`; ActivityWatch can be chosen only after a successful connection test, and saving a new source appends an `ActivitySourceSelection` and recalculates the day, with French and English strings
- [x] T128 [US8] Add the built-in source health states (recording, input query failed) and show ActivityWatch notices only while ActivityWatch is selected, never suggesting a switch automatically, in `src/WorkTime.App/Presentation/Tray/TrayViewModel.cs`, `src/WorkTime.App/Views/WorkdayFlyout.xaml`, and the French and English resources
- [x] T129 [US10] Remove the blocking ActivityWatch step from onboarding: the built-in source is selected by default, confirmation appends the first `ActivitySourceSelection` effective at the confirmation instant before saving the settings, and a background probe of the default address offers ActivityWatch only when it is detected, in `src/WorkTime.App/Services/Onboarding/` and `src/WorkTime.App/Views/Onboarding/`, with updated cases in `tests/WorkTime.App.Tests/Onboarding/OnboardingTests.cs`
- [x] T130 [P] [US10] Update the as-built guidance for the built-in source in `docs/architecture.md`, `docs/privacy.md`, `docs/troubleshooting.md`, `README.md`, and `CONTRIBUTING.md`, and point the demo, screenshot, and performance scripts at the built-in source where they assume ActivityWatch
- [ ] T131 [US10] Run the built-in source smoke on Windows 11 x64 (real input moves the source between active and idle, crash and restart leave an unknown gap, a clean exit within the idle threshold leaves the time after the last input unknown, a source switch splits the day, and a fresh install leaves the time before onboarding unknown) and record it in `docs/built-in-activity-source-validation.md`

**Checkpoint**: WorkTime installs and counts work time on a machine without
ActivityWatch, and an existing ActivityWatch user keeps their source.

---

## Dependencies & Execution Order

### Phase dependencies

- Setup → Foundational → all user stories.
- US1 is the first runnable tray MVP.
- US2 depends on US1 summary/tray presentation.
- US3 depends on foundational source/persistence ports and enables live data.
- US5 depends on US3 accounting and session persistence.
- US4 depends only on Foundational, but follows P1 delivery in the recommended
  sequence.
- US6 depends on US4 settings and US5 adjustments/recalculation.
- US7 depends on US2 summaries and US4 notification settings.
- US8 depends on US3 ActivityWatch integration.
- US9 depends on US5 adjustments and US6 credit.
- US10 depends on US3 source health, US4 settings, US7 notifications, and US8
  diagnostics.
- Phase 14 (built-in activity source) depends on US3, US4, US8, and US10.
  T120-T125 come before T126-T128, which come before T129-T131.
- Polish depends on all stories included in the release. The open release
  validations (T108, T116, T117) run after Phase 14, so they cover the
  built-in source and onboarding without ActivityWatch.

### User-story dependency graph

```text
US1 → US2 ───────────────→ US7 ─┐
  └→ US3 → US5 → US6 → US9     ├→ US10 → Polish
          └────→ US8 ──────────┘
Foundational → US4 → US6/US7/US10
```

### Parallel opportunities

- Setup: T003–T005, T007, and T008 can proceed after T001/T002.
- Foundational: time/domain, persistence, ports, fixtures, and logging have
  separate files and can proceed in parallel as marked.
- US3: Rust/Python fixtures and Windows session/power adapters are parallel.
- US4: the two settings page groups are parallel after validation contracts.
- US5: domain tests and distinct views/commands are parallel as marked.
- US7: domain eligibility, receipt persistence, Windows adapter, and resources
  are parallel.
- US9: day and week views are parallel.
- US10: localization, secret, onboarding, and update tests are parallel.

## Parallel Examples

### US1

```text
T023 IndicatorStateTests.cs
T024 TrayPresentationTests.cs
T027 WorkTime.Windows/Tray/
T028 WorkTime.App/Assets/Tray/
```

### US3

```text
T036 Fixtures/Rust/
T037 Fixtures/Python/
T043 WorkTime.Windows/Sessions/
T044 WorkTime.Windows/Power/
```

### US5

```text
T048 AdjustmentTests.cs
T049 TrackingControlTests.cs
T050 LongLockReviewTests.cs
T055 AdjustTimeDialog.xaml
T056 TrayCommandViewModel.cs
```

### US7

```text
T073 NotificationEligibilityTests.cs
T074 NotificationReceiptTests.cs
T077 WindowsNotificationService.cs
T079 localized notification resources
```

### US10

```text
T095 OnboardingTests.cs
T096 DpapiSecretStoreTests.cs
T097 UpdateCheckTests.cs
T101/T102 French and English resources
```

## Implementation Strategy

### Demo MVP

1. Complete Setup and Foundational.
2. Complete US1.
3. Validate deterministic tray state independently.

### Useful local MVP

1. Add US2 for forecast/overtime.
2. Add US3 for live ActivityWatch/Windows accounting.
3. Add US5 for safe corrections and finish-day control.
4. Stop and validate daily use before expanding breadth.

### Public v1

Complete US4, US6–US10, Phase 14, then all Polish/release gates.

## Format Validation

Every implementation line uses:

```text
- [ ] T### [P?] [US#?] Action with exact repository file path
```

Tests precede implementation within each story where behavioural risk requires
them. Commit after each task or coherent task group and validate the story
checkpoint before proceeding.
