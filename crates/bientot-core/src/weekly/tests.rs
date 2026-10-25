//! Ported from `WeeklyRecalculationServiceTests.cs`, against a migrated SQLite ledger. The tests
//! that observe the refresh coordinator's publications refresh through [`RefreshCoordinator`].
//! The tests on the ActivityWatch bucket selection or its polling service are reduced to what this
//! service decides; each test below says how.
//!
//! Left out: `CancellationAtCommitWritesAndPublishesNothing` and
//! `CancellationDuringHistoricalCommitLeavesTheOldAndCurrentWeeksUnchanged`. A recalculation
//! runs to its end once started, and commits each week in one transaction; a refresh superseded
//! meanwhile publishes nothing, which `refresh/tests.rs` checks.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use bientot_domain::accounting::{
    AccountingLabel, ActivityCoverage, ActivitySegment, ActivityState, AdjustmentId,
    AutomaticOrigin, OperationKey, SessionState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::adjustments::{ManualAdjustment, ManualClassification};
use bientot_domain::configuration::Settings;
use bientot_domain::credit::{CreditLedgerEntryKind, CreditOrigin};
use bientot_domain::forecasting::AssessmentReason;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::{
    ActivitySourceKind, ActivitySourceSelection, SourceFreshness, SourceHealth, SourceQuality,
};
use bientot_domain::summaries::IndicatorState;
use bientot_domain::time::{Clock, FrozenClock, TimeZoneProvider};
use bientot_store::StorageError;
use bientot_store::activity::{ActivityError, ActivitySourceSelectionRepository};
use bientot_store::adjustments::ManualAdjustmentRepository;
use bientot_store::sessions::SessionEventRepository;
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span};

use super::*;
use crate::refresh::RefreshCoordinator;
use crate::routing::ActivitySourceRouter;
use crate::sources::{ActivityReadResult, ActivitySource};
use crate::test_support::{Database, utc_zones};

const MONDAY: Date = date(2026, 9, 7);
const SOURCE_ID: &str = "activitywatch:test";

fn day(offset: i64) -> Date {
    MONDAY.checked_add(Span::new().days(offset)).unwrap()
}

fn tuesday() -> Date {
    day(1)
}

fn at(date: Date, hour: i8) -> Timestamp {
    date.at(hour, 0, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn hours(hours: f64) -> SignedDuration {
    SignedDuration::from_secs_f64(hours * 3600.0)
}

/// Monday to Friday of the week of `monday`, each worked the hours given for it.
fn workweek(monday: Date, hours_by_day: [f64; 5]) -> Vec<(Date, SignedDuration)> {
    (0..5)
        .map(|offset| {
            (
                monday.checked_add(Span::new().days(offset)).unwrap(),
                hours(hours_by_day[offset as usize]),
            )
        })
        .collect()
}

/// The .NET `MutableActivitySource`: the day a read belongs to is active from 08:00 into the read
/// for its configured work, and covered as away around it when `complete_coverage` is set.
struct WorkSource {
    work: RefCell<HashMap<Date, SignedDuration>>,
    unavailable: RefCell<HashSet<Date>>,
    stale: RefCell<HashSet<Date>>,
    complete_coverage: bool,
    date_selector: fn(UtcInterval) -> Date,
    requests: RefCell<Vec<UtcInterval>>,
}

impl WorkSource {
    fn new(work: impl IntoIterator<Item = (Date, SignedDuration)>) -> Self {
        Self {
            work: RefCell::new(work.into_iter().collect()),
            unavailable: RefCell::default(),
            stale: RefCell::default(),
            complete_coverage: false,
            date_selector: start_date,
            requests: RefCell::default(),
        }
    }

    fn complete(self) -> Self {
        Self {
            complete_coverage: true,
            ..self
        }
    }

    /// Reads that start at midnight UTC belong to their start date, Paris windows to their end
    /// date.
    fn utc_or_paris(self) -> Self {
        Self {
            date_selector: |request| {
                if request.start().to_zoned(TimeZone::UTC).time() == jiff::civil::Time::midnight() {
                    start_date(request)
                } else {
                    request.end().to_zoned(TimeZone::UTC).date()
                }
            },
            ..self
        }
    }

    fn request_count(&self) -> usize {
        self.requests.borrow().len()
    }
}

fn start_date(request: UtcInterval) -> Date {
    request.start().to_zoned(TimeZone::UTC).date()
}

impl ActivitySource for WorkSource {
    fn read(&self, request: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        self.requests.borrow_mut().push(request);
        let date = (self.date_selector)(request);
        if self.unavailable.borrow().contains(&date) {
            return Ok(ActivityReadResult::Unavailable {
                health: SourceHealth::degraded(
                    SOURCE_ID,
                    SourceFreshness::Unavailable,
                    SourceQuality::Incomplete,
                    "test outage",
                    None,
                )
                .unwrap(),
            });
        }
        let active = self
            .work
            .borrow()
            .get(&date)
            .copied()
            .unwrap_or(SignedDuration::ZERO);
        let active_start = request.start() + SignedDuration::from_hours(8);
        let active_end = active_start + active;
        let origin = || AutomaticOrigin::new(SOURCE_ID).unwrap();
        let mut segments = Vec::new();
        if self.complete_coverage && request.start() < active_start {
            segments.push(ActivitySegment::new(
                UtcInterval::new(request.start(), active_start).unwrap(),
                ActivityState::Afk,
                origin(),
            ));
        }
        if active.is_positive() && active_end <= request.end() {
            segments.push(ActivitySegment::new(
                UtcInterval::new(active_start, active_end).unwrap(),
                ActivityState::Active,
                origin(),
            ));
        }
        if self.complete_coverage && active_end < request.end() {
            segments.push(ActivitySegment::new(
                UtcInterval::new(active_end, request.end()).unwrap(),
                ActivityState::Afk,
                origin(),
            ));
        }
        let health = if self.stale.borrow().contains(&date) {
            SourceHealth::degraded(
                SOURCE_ID,
                SourceFreshness::Stale,
                SourceQuality::Incomplete,
                "retained test snapshot",
                Some(request.end()),
            )
            .unwrap()
        } else {
            SourceHealth::healthy(SOURCE_ID, request.end())
        };
        Ok(ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(request, segments),
            health,
        })
    }
}

/// The local zone first, then the other zones it knows, as the .NET `CatalogTimeZoneProvider`.
struct CatalogZones {
    zones: Vec<(&'static str, TimeZone)>,
}

impl CatalogZones {
    fn paris_with_utc() -> Self {
        Self {
            zones: vec![
                ("Europe/Paris", TimeZone::get("Europe/Paris").unwrap()),
                ("UTC", TimeZone::UTC),
            ],
        }
    }
}

impl TimeZoneProvider for CatalogZones {
    fn local(&self) -> (String, TimeZone) {
        let (id, zone) = &self.zones[0];
        ((*id).to_owned(), zone.clone())
    }

    fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
        self.zones
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, zone)| zone.clone())
            .ok_or_else(|| UnknownTimeZone(id.to_owned()))
    }
}

/// A ledger and the clock every service of a test reads.
struct Harness {
    database: Database,
    clock: FrozenClock,
}

impl Harness {
    fn new(now: Timestamp) -> Self {
        Self {
            database: Database::new(),
            clock: FrozenClock::new(now),
        }
    }

    fn store(&self) -> WeeklyCreditRepository<'_> {
        WeeklyCreditRepository::new(&self.database.connection)
    }

    fn week(&self, monday: Date) -> WeeklyCreditState {
        self.store().read_week(monday).unwrap()
    }

    fn execute(&self, sql: &str) {
        self.database.connection.execute_batch(sql).unwrap();
    }

    /// Runs `run` with a weekly recalculation over `source` and `store`, in the zones and with
    /// the settings given.
    fn with_weekly<R>(
        &self,
        source: &dyn ActivitySource,
        store: &dyn WeeklyCreditStore,
        settings: &Settings,
        zones: &dyn TimeZoneProvider,
        run: impl FnOnce(&WeeklyRecalculation) -> R,
    ) -> R {
        let read_settings = || settings.clone();
        let accounting = WorkdayAccounting::new(
            source,
            SessionEventRepository::new(&self.database.connection, &self.clock),
            ManualAdjustmentRepository::new(&self.database.connection),
            &self.clock,
            zones,
            &read_settings,
        );
        run(&WeeklyRecalculation::new(&accounting, store))
    }

    fn current_with(
        &self,
        source: &dyn ActivitySource,
        store: &dyn WeeklyCreditStore,
        settings: &Settings,
        zones: &dyn TimeZoneProvider,
    ) -> Result<DailySummary, WeeklyError> {
        self.with_weekly(source, store, settings, zones, |weekly| {
            weekly.calculate_current()
        })
    }

    /// Today's summary in UTC with the default settings, committed to the ledger.
    fn current(&self, source: &dyn ActivitySource) -> Result<DailySummary, WeeklyError> {
        self.current_with(source, &self.store(), &Settings::default(), &utc_zones())
    }

    fn refresh(
        &self,
        source: &dyn ActivitySource,
        store: &dyn WeeklyCreditStore,
        previous: Option<&WorkdayRefreshState>,
    ) -> WorkdayRefreshState {
        self.with_weekly(
            source,
            store,
            &Settings::default(),
            &utc_zones(),
            |weekly| weekly.refresh(previous).unwrap(),
        )
    }
}

fn revisions(state: &WeeklyCreditState) -> [i64; 5] {
    state.revision_vector().revisions()
}

fn head(state: &WeeklyCreditState, date: Date) -> &DailySummaryRevision {
    state
        .latest_revisions()
        .iter()
        .find(|revision| revision.workday_date() == date)
        .unwrap()
}

fn expiries(state: &WeeklyCreditState) -> Vec<&CreditLedgerEntry> {
    state
        .ledger()
        .iter()
        .filter(|entry| entry.kind() == CreditLedgerEntryKind::Expired)
        .collect()
}

fn single_expiry(state: &WeeklyCreditState) -> &CreditLedgerEntry {
    let [expiry] = expiries(state)[..] else {
        panic!("expected one expiry in {:?}", state.ledger());
    };
    expiry
}

fn source_unavailable(error: WeeklyError) -> SourceEvidenceUnavailable {
    match error {
        WeeklyError::SourceEvidenceUnavailable(failure) => *failure,
        error => panic!("expected missing source evidence, got {error:?}"),
    }
}

/// Counts commits and passes everything through.
struct CountingStore<'a> {
    inner: &'a dyn WeeklyCreditStore,
    commits: Cell<usize>,
}

impl<'a> CountingStore<'a> {
    fn new(inner: &'a dyn WeeklyCreditStore) -> Self {
        Self {
            inner,
            commits: Cell::new(0),
        }
    }
}

impl WeeklyCreditStore for CountingStore<'_> {
    fn read_pending_expiry_weeks(&self, before: Date) -> Result<Vec<Date>, WeeklyCreditError> {
        self.inner.read_pending_expiry_weeks(before)
    }

    fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError> {
        self.inner.read_week(monday)
    }

    fn commit(
        &self,
        monday: Date,
        expected: &WeeklyRevisionVector,
        revisions: &[DailySummaryRevision],
        ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        self.commits.set(self.commits.get() + 1);
        self.inner.commit(monday, expected, revisions, ledger)
    }
}

/// Commits the first commit, then reports it stale as if another writer had won the race.
struct CompetingCommitStore<'a> {
    inner: WeeklyCreditRepository<'a>,
    after_first_commit: Box<dyn Fn() + 'a>,
    reads: Cell<usize>,
    commits: Cell<usize>,
}

impl<'a> CompetingCommitStore<'a> {
    fn new(inner: WeeklyCreditRepository<'a>, after_first_commit: impl Fn() + 'a) -> Self {
        Self {
            inner,
            after_first_commit: Box::new(after_first_commit),
            reads: Cell::new(0),
            commits: Cell::new(0),
        }
    }
}

impl WeeklyCreditStore for CompetingCommitStore<'_> {
    fn read_pending_expiry_weeks(&self, before: Date) -> Result<Vec<Date>, WeeklyCreditError> {
        self.inner.read_pending_expiry_weeks(before)
    }

    fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError> {
        self.reads.set(self.reads.get() + 1);
        self.inner.read_week(monday)
    }

    fn commit(
        &self,
        monday: Date,
        expected: &WeeklyRevisionVector,
        revisions: &[DailySummaryRevision],
        ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        self.commits.set(self.commits.get() + 1);
        self.inner.commit(monday, expected, revisions, ledger)?;
        if self.commits.get() == 1 {
            let actual = self.inner.read_revision_vector(monday)?;
            (self.after_first_commit)();
            return Err(WeeklyCreditError::Stale {
                expected: *expected,
                actual,
            });
        }
        Ok(())
    }
}

/// An empty week whose every commit is stale.
struct AlwaysStaleStore<'a> {
    empty: WeeklyCreditRepository<'a>,
    reads: Cell<usize>,
    commits: Cell<usize>,
}

impl WeeklyCreditStore for AlwaysStaleStore<'_> {
    fn read_pending_expiry_weeks(&self, _: Date) -> Result<Vec<Date>, WeeklyCreditError> {
        Ok(Vec::new())
    }

    fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError> {
        self.reads.set(self.reads.get() + 1);
        self.empty.read_week(monday)
    }

    fn commit(
        &self,
        _: Date,
        expected: &WeeklyRevisionVector,
        _: &[DailySummaryRevision],
        _: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        self.commits.set(self.commits.get() + 1);
        Err(WeeklyCreditError::Stale {
            expected: *expected,
            actual: *expected,
        })
    }
}

/// Reports the first `stale_attempts` commits of `historical_week` stale.
struct HistoricalStaleStore<'a> {
    inner: WeeklyCreditRepository<'a>,
    historical_week: Date,
    stale_attempts: usize,
    reads: Cell<usize>,
    commits: Cell<usize>,
}

impl WeeklyCreditStore for HistoricalStaleStore<'_> {
    fn read_pending_expiry_weeks(&self, before: Date) -> Result<Vec<Date>, WeeklyCreditError> {
        self.inner.read_pending_expiry_weeks(before)
    }

    fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError> {
        if monday == self.historical_week {
            self.reads.set(self.reads.get() + 1);
        }
        self.inner.read_week(monday)
    }

    fn commit(
        &self,
        monday: Date,
        expected: &WeeklyRevisionVector,
        revisions: &[DailySummaryRevision],
        ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        if monday == self.historical_week {
            self.commits.set(self.commits.get() + 1);
            if self.commits.get() <= self.stale_attempts {
                return Err(WeeklyCreditError::Stale {
                    expected: *expected,
                    actual: *expected,
                });
            }
        }
        self.inner.commit(monday, expected, revisions, ledger)
    }
}

/// .NET resolves the overnight lock through `ManualAdjustmentCommandService`, which R5 ports
/// with the corrections; the adjustments are appended to the ledger directly here.
/// Not in the .NET tests: .NET durations have no precision below the 100 ns tick the database
/// keeps.
#[test]
fn an_unchanged_recalculation_with_sub_tick_values_keeps_its_revision() {
    let harness = Harness::new(at(MONDAY, 18));
    let source = WorkSource::new([(MONDAY, hours(8.0))]).complete();
    let settings = Settings {
        weekly_credit_enabled: false,
        nominal_daily_target: hours(7.0) + SignedDuration::from_nanos(1),
        ..Settings::default()
    };
    assert!(settings.validate().is_empty());
    let store = harness.store();

    let first = harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();
    let committed = harness.week(MONDAY);
    let second = harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();

    assert_eq!(second.credited_work(), first.credited_work());
    assert_eq!(committed.revision_vector().revisions(), [1, 0, 0, 0, 0]);
    assert_eq!(
        harness.week(MONDAY).latest_revisions(),
        committed.latest_revisions()
    );
}

#[test]
fn overnight_lock_is_excluded_until_declared_as_work() {
    for declare_work in [false, true] {
        let harness = Harness::new(at(tuesday(), 18));
        let events = SessionEventRepository::new(&harness.database.connection, &harness.clock);
        for (key, instant, state) in [
            ("evening-lock", at(MONDAY, 18), SessionState::Locked),
            ("morning-unlock", at(tuesday(), 9), SessionState::Unlocked),
        ] {
            events
                .record(
                    OperationKey::new(key).unwrap(),
                    WorkdayTransition::new(
                        instant,
                        "UTC",
                        WorkdayTransitionKind::SessionObserved(state),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let source = WorkSource::new([(MONDAY, hours(7.0)), (tuesday(), hours(7.0))]).complete();

        let summary = harness.current(&source).unwrap();
        let before = harness.week(MONDAY);

        assert_eq!(summary.credited_work(), hours(6.0));
        assert_eq!(
            summary.accounting().work_awaiting_review(),
            SignedDuration::ZERO
        );
        assert_eq!(
            summary.credit_result().unwrap().closing_balance(),
            SignedDuration::ZERO
        );
        assert_eq!(summary.applied_carry_credit(), SignedDuration::ZERO);
        assert!(
            !summary
                .assessment()
                .reasons()
                .contains(&AssessmentReason::ProvisionalLockReview)
        );
        assert!(summary.accounting().pending_reviews().is_empty());
        let overnight: Vec<_> = summary
            .accounting()
            .timeline()
            .segments()
            .iter()
            .filter(|segment| segment.label() == AccountingLabel::OvernightLock)
            .map(|segment| segment.interval())
            .collect();
        assert_eq!(
            overnight,
            [UtcInterval::new(at(tuesday(), 0), at(tuesday(), 9)).unwrap()]
        );
        assert!(
            before
                .latest_revisions()
                .iter()
                .all(|revision| !revision.snapshot().has_pending_review())
        );
        assert!(before.ledger().is_empty());

        let adjustments = ManualAdjustmentRepository::new(&harness.database.connection);
        for date in [MONDAY, tuesday()] {
            let interval = if date == MONDAY {
                UtcInterval::new(at(MONDAY, 18), at(tuesday(), 0)).unwrap()
            } else {
                UtcInterval::new(at(tuesday(), 0), at(tuesday(), 9)).unwrap()
            };
            let id = AdjustmentId::new(format!("resolve-{}", date.strftime("%Y%m%d"))).unwrap();
            let workday = LocalWorkday::new(date, "UTC");
            let now = harness.clock.now();
            let adjustment = if declare_work {
                ManualAdjustment::interval_work(id, workday, interval, now)
            } else {
                ManualAdjustment::reclassification(
                    id,
                    workday,
                    interval,
                    ManualClassification::Excluded,
                    now,
                )
            };
            adjustments.append(&adjustment.unwrap()).unwrap();
        }

        let summary = harness.current(&source).unwrap();
        let resolved = harness.week(MONDAY);
        assert_eq!(
            summary.credit_result().unwrap().closing_balance(),
            if declare_work {
                hours(14.0)
            } else {
                SignedDuration::ZERO
            }
        );
        assert_eq!(
            summary.accounting().work_awaiting_review(),
            SignedDuration::ZERO
        );
        assert!(
            !summary
                .assessment()
                .reasons()
                .contains(&AssessmentReason::ProvisionalLockReview)
        );
        assert!(
            resolved
                .latest_revisions()
                .iter()
                .all(|revision| !revision.snapshot().has_pending_review()
                    && revision.revision() == 2)
        );

        let reopened_connection = harness.database.reopen();
        let reopened = WeeklyCreditRepository::new(&reopened_connection);
        assert_eq!(
            reopened.read_week(MONDAY).unwrap().ledger(),
            resolved.ledger()
        );
        harness
            .current_with(&source, &reopened, &Settings::default(), &utc_zones())
            .unwrap();
        let replayed = reopened.read_week(MONDAY).unwrap();
        assert_eq!(revisions(&replayed), revisions(&resolved));
        assert_eq!(replayed.ledger(), resolved.ledger());
    }
}

/// A refresh returns once its week is committed, so the publication always follows the commit.
#[test]
fn monday_overtime_reduces_tuesday_target_and_publishes_after_the_commit() {
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([(MONDAY, hours(8.0)), (tuesday(), hours(6.0))]);
    let mut coordinator = RefreshCoordinator::new();

    let publication = coordinator
        .refresh(|_| harness.current(&source))
        .unwrap()
        .clone();
    let summary = publication.summary;

    let state = harness.week(MONDAY);
    assert_eq!(revisions(&state), [1, 1, 0, 0, 0]);
    let movements: Vec<_> = state
        .ledger()
        .iter()
        .map(|entry| {
            (
                entry.kind(),
                entry.workday_date(),
                entry.origin(),
                entry.amount(),
            )
        })
        .collect();
    let origin = CreditOrigin {
        workday_date: MONDAY,
        revision: 1,
    };
    assert_eq!(
        movements,
        [
            (CreditLedgerEntryKind::Earned, MONDAY, origin, hours(1.0)),
            (
                CreditLedgerEntryKind::Applied,
                tuesday(),
                origin,
                hours(1.0)
            ),
        ]
    );
    assert_eq!(publication.revision, 1);
    assert_eq!(summary.effective_target(), hours(6.0));
    assert_eq!(summary.indicator_state(), IndicatorState::Blue);
}

/// The .NET immediate refresh also bypasses the ActivityWatch polling backoff, which v1 does not
/// have.
#[test]
fn operational_clock_changes_reuse_exact_revisions() {
    let saturday = day(5);
    let harness = Harness::new(at(saturday, 10));
    let source = WorkSource::new(workweek(MONDAY, [7.0; 5]));

    harness.current(&source).unwrap();
    let first = harness.week(MONDAY);
    let first_calls = source.request_count();
    harness.clock.set(at(saturday, 11));
    harness.current(&source).unwrap();
    let second = harness.week(MONDAY);

    assert_eq!(revisions(&second), revisions(&first));
    assert_eq!(second.ledger(), first.ledger());
    assert_eq!(source.request_count() - first_calls, 6);
}

#[test]
fn restart_outage_with_changed_lunch_preserves_whole_week_until_recovery() {
    let harness = Harness::new(at(tuesday(), 18));
    let work = [(MONDAY, hours(8.0)), (tuesday(), hours(6.0))];
    harness.current(&WorkSource::new(work)).unwrap();
    let before = harness.week(MONDAY);
    let changed = Settings {
        lunch_start: jiff::civil::time(12, 30, 0, 0),
        lunch_end: jiff::civil::time(14, 30, 0, 0),
        ..Settings::default()
    };
    let restarted = WorkSource::new(work);
    restarted.unavailable.borrow_mut().insert(MONDAY);
    let store = harness.store();

    let failure = source_unavailable(
        harness
            .current_with(&restarted, &store, &changed, &utc_zones())
            .unwrap_err(),
    );

    assert_eq!(failure.workday_date, MONDAY);
    assert_eq!(failure.disposition, ActivityReadDisposition::Unavailable);
    assert_eq!(
        failure.to_string(),
        "The source has no usable evidence for workday 2026-09-07 (Unavailable)."
    );
    let failed = harness.week(MONDAY);
    assert_eq!(revisions(&failed), revisions(&before));
    assert_eq!(failed.ledger(), before.ledger());
    assert_eq!(restarted.request_count(), 1);

    restarted.unavailable.borrow_mut().clear();
    harness
        .current_with(&restarted, &store, &changed, &utc_zones())
        .unwrap();
    assert_eq!(revisions(&harness.week(MONDAY)), [2, 2, 0, 0, 0]);
}

/// In .NET the failure is a bucket selection, and the test also checks the notice, tray and
/// notifications. V1 has no bucket to select, so the source is unavailable instead, and the
/// presentation is ported with the screens.
#[test]
fn source_failure_retains_committed_week_and_recovers() {
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([(MONDAY, hours(8.0)), (tuesday(), hours(6.0))]).complete();
    let ledger = harness.store();
    let store = CountingStore::new(&ledger);

    let healthy = harness.refresh(&source, &store, None);
    let before = harness.week(MONDAY);
    let commits_before_failure = store.commits.get();
    assert!(healthy.summary.is_some());
    assert_eq!(healthy.source_failure, None);

    source.unavailable.borrow_mut().insert(MONDAY);
    let failed = harness.refresh(&source, &store, Some(&healthy));

    let after_failure = harness.week(MONDAY);
    assert_eq!(store.commits.get(), commits_before_failure);
    assert_eq!(revisions(&after_failure), revisions(&before));
    assert_eq!(after_failure.ledger(), before.ledger());
    assert_eq!(failed.summary, healthy.summary);
    let failure = failed.source_failure.as_ref().unwrap();
    assert_eq!(failure.workday_date, MONDAY);
    assert_eq!(failure.disposition, ActivityReadDisposition::Unavailable);

    source.unavailable.borrow_mut().clear();
    let recovered = harness.refresh(&source, &store, Some(&failed));

    assert_eq!(recovered.source_failure, None);
    assert!(recovered.summary.is_some());
}

/// A first source selection on Wednesday leaves Monday and Tuesday without a source: their time
/// stays unknown, every later refresh of the week still commits, and the following Monday
/// closes the week.
#[test]
fn days_before_the_first_source_selection_neither_block_refreshes_nor_the_week_closure() {
    let wednesday = day(2);
    let thursday = day(3);
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(wednesday, 23));
    let selections =
        ActivitySourceSelectionRepository::new(&harness.database.connection, &harness.clock);
    selections
        .append(ActivitySourceSelection {
            effective_from: at(wednesday, 6),
            source: ActivitySourceKind::BuiltIn,
        })
        .unwrap();
    let built_in = WorkSource::new([
        (wednesday, hours(8.0)),
        (thursday, hours(8.0)),
        (friday, hours(9.0)),
        (following_monday, hours(7.0)),
    ])
    .complete();
    let router = ActivitySourceRouter::new(selections, &built_in);

    let first = harness.refresh(&router, &harness.store(), None);
    harness.clock.set(at(thursday, 23));
    let second = harness.refresh(&router, &harness.store(), Some(&first));
    harness.clock.set(at(friday, 23));
    let third = harness.refresh(&router, &harness.store(), Some(&second));
    harness.clock.set(at(following_monday, 18));
    let closing = harness.refresh(&router, &harness.store(), Some(&third));

    for state in [&first, &second, &third, &closing] {
        assert_eq!(state.source_failure, None);
    }
    let week = harness.week(MONDAY);
    assert_eq!(
        head(&week, MONDAY).snapshot().unknown_work(),
        SignedDuration::from_hours(24)
    );
    assert_eq!(revisions(&week), [1, 1, 2, 2, 2]);
    let closed_at = at(following_monday, 18);
    assert_eq!(
        expiries(&week)
            .iter()
            .map(|expiry| (expiry.workday_date(), expiry.amount(), expiry.created_at()))
            .collect::<Vec<_>>(),
        [
            (friday, hours(3.0), closed_at),
            (friday, hours(1.0), closed_at)
        ]
    );
    assert_eq!(
        harness
            .store()
            .read_pending_expiry_weeks(following_monday)
            .unwrap(),
        []
    );
}

/// Adapted like the test above: an unavailable source in place of a bucket selection.
#[test]
fn initial_source_failure_with_persisted_data_does_not_fabricate_summary() {
    let harness = Harness::new(at(tuesday(), 18));
    let work = [(MONDAY, hours(8.0)), (tuesday(), hours(6.0))];
    harness.current(&WorkSource::new(work)).unwrap();
    let before = harness.week(MONDAY);
    let source = WorkSource::new(work);
    source.unavailable.borrow_mut().insert(MONDAY);
    let ledger = harness.store();
    let store = CountingStore::new(&ledger);

    let state = harness.refresh(&source, &store, None);

    let after = harness.week(MONDAY);
    assert_eq!(state.summary, None);
    assert_eq!(state.source_failure.unwrap().workday_date, MONDAY);
    assert_eq!(revisions(&after), revisions(&before));
    assert_eq!(after.ledger(), before.ledger());
    assert_eq!(store.commits.get(), 0);
}

/// `SupersededSelectionFailureAndCallerCancellationPublishNothing`, with an unavailable source in
/// place of the bucket selection: a failing refresh that a recovery superseded publishes nothing,
/// even when it finishes last. The caller's cancellation is `an_abandoned_refresh_publishes_
/// nothing` in `refresh/tests.rs`.
#[test]
fn a_superseded_source_failure_publishes_nothing() {
    let harness = Harness::new(at(tuesday(), 18));
    let work = [(MONDAY, hours(8.0)), (tuesday(), hours(6.0))];
    harness.current(&WorkSource::new(work)).unwrap();
    let failing = WorkSource::new(work);
    failing.unavailable.borrow_mut().insert(MONDAY);
    let recovered = WorkSource::new(work);
    let store = harness.store();
    let mut coordinator = RefreshCoordinator::<WorkdayRefreshState>::new();

    let delayed_failure = coordinator.begin();
    let recovery = coordinator.begin();
    let recovered_state = harness.refresh(&recovered, &store, None);
    let failed_state = harness.refresh(&failing, &store, None);
    let published = coordinator
        .complete(recovery, Ok::<_, WeeklyError>(recovered_state))
        .unwrap()
        .cloned()
        .unwrap();

    assert_eq!(
        failed_state.source_failure.as_ref().unwrap().workday_date,
        MONDAY
    );
    assert_eq!(
        coordinator
            .complete(delayed_failure, Ok::<_, WeeklyError>(failed_state))
            .unwrap(),
        None
    );
    assert_eq!(published.revision, 1);
    assert_eq!(published.summary.source_failure, None);
    assert_eq!(coordinator.current(), Some(&published));
}

#[test]
fn retrospective_monday_correction_revises_affected_days_and_reuses_unaffected_head() {
    let wednesday = day(2);
    let harness = Harness::new(at(wednesday, 18));
    let source = WorkSource::new([
        (MONDAY, hours(8.0)),
        (tuesday(), hours(6.0)),
        (wednesday, hours(7.0)),
    ]);
    harness.current(&source).unwrap();
    let calls_after_first_run = source.request_count();

    source.work.borrow_mut().insert(MONDAY, hours(7.0));
    harness.current(&source).unwrap();
    let corrected = harness.week(MONDAY);

    assert_eq!(source.request_count() - calls_after_first_run, 3);
    assert_eq!(revisions(&corrected), [2, 2, 1, 0, 0]);
    assert!(corrected.ledger().is_empty());
}

#[test]
fn ended_days_persist_debt_the_running_day_adds_none_and_a_correction_recalculates_it() {
    let wednesday = day(2);
    let harness = Harness::new(at(wednesday, 18));
    let source = WorkSource::new([
        (MONDAY, hours(6.0)),
        (tuesday(), hours(7.5)),
        (wednesday, hours(5.0)),
    ]);
    let settings = Settings {
        weekly_debt_enabled: true,
        ..Settings::default()
    };
    let store = harness.store();

    let current = harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();
    let state = harness.week(MONDAY);

    assert_eq!(head(&state, MONDAY).snapshot().carry_debt(), hours(1.0));
    assert_eq!(head(&state, tuesday()).snapshot().carry_debt(), hours(0.5));
    assert_eq!(head(&state, wednesday).snapshot().carry_debt(), hours(0.5));
    assert_eq!(
        head(&state, wednesday).snapshot().net_carry_balance(),
        hours(-0.5)
    );
    assert_eq!(current.effective_target(), hours(7.0));
    assert_eq!(current.credit_result().unwrap().closing_debt(), hours(0.5));
    assert!(state.ledger().is_empty());

    source.work.borrow_mut().insert(MONDAY, hours(7.0));
    harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();
    let corrected = harness.week(MONDAY);

    assert!(
        corrected
            .latest_revisions()
            .iter()
            .all(|revision| revision.snapshot().carry_debt().is_zero())
    );
    assert_eq!(
        head(&corrected, tuesday()).snapshot().carry_earned(),
        hours(0.5)
    );
    assert_eq!(
        head(&corrected, wednesday).snapshot().carry_applied(),
        hours(0.5)
    );
    assert_eq!(corrected.ledger().len(), 2);
}

#[test]
fn following_monday_finalizes_the_debt_of_a_friday_left_running_without_credit() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 18));
    let mut work = workweek(MONDAY, [7.0, 7.0, 7.0, 7.0, 6.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    let settings = Settings {
        weekly_debt_enabled: true,
        ..Settings::default()
    };
    let store = harness.store();

    harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();
    let running = harness.week(MONDAY);
    let running = head(&running, friday).snapshot();
    assert!(running.debt_pending());
    assert_eq!(running.carry_debt(), SignedDuration::ZERO);
    assert_eq!(running.carry_available(), SignedDuration::ZERO);
    assert_eq!(
        store.read_pending_expiry_weeks(following_monday).unwrap(),
        [MONDAY]
    );

    harness.clock.set(at(following_monday, 18));
    let monday_summary = harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();
    let closed = harness.week(MONDAY);
    let friday_revision = head(&closed, friday);
    assert!(!friday_revision.snapshot().debt_pending());
    assert_eq!(friday_revision.snapshot().carry_debt(), hours(1.0));
    let credit = monday_summary.credit_result().unwrap();
    assert_eq!(credit.opening_debt(), SignedDuration::ZERO);
    assert_eq!(credit.closing_debt(), SignedDuration::ZERO);
    assert!(
        store
            .read_pending_expiry_weeks(following_monday)
            .unwrap()
            .is_empty()
    );

    harness
        .current_with(&source, &store, &settings, &utc_zones())
        .unwrap();
    assert_eq!(
        harness
            .week(MONDAY)
            .revision_vector()
            .revision(friday)
            .unwrap(),
        friday_revision.revision()
    );
}

#[test]
fn debt_off_persists_no_debt_and_reuses_the_same_revisions() {
    let wednesday = day(2);
    let harness = Harness::new(at(wednesday, 18));
    let source = WorkSource::new([
        (MONDAY, hours(6.0)),
        (tuesday(), hours(8.0)),
        (wednesday, hours(5.0)),
    ]);

    harness.current(&source).unwrap();
    let first = harness.week(MONDAY);
    harness.current(&source).unwrap();
    let second = harness.week(MONDAY);

    assert!(
        first
            .latest_revisions()
            .iter()
            .all(|revision| revision.snapshot().carry_debt().is_zero())
    );
    assert_eq!(
        head(&first, tuesday()).snapshot().carry_earned(),
        hours(1.0)
    );
    assert_eq!(revisions(&second), revisions(&first));
}

#[test]
fn competing_commit_causes_full_coherent_retry_and_one_publication() {
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([(MONDAY, hours(8.0)), (tuesday(), hours(6.0))]);
    let competing = CompetingCommitStore::new(harness.store(), || {});
    let mut coordinator = RefreshCoordinator::new();

    coordinator
        .refresh(|_| harness.current_with(&source, &competing, &Settings::default(), &utc_zones()))
        .unwrap();

    assert_eq!(coordinator.current().unwrap().revision, 1);

    assert_eq!(competing.reads.get(), 2);
    assert_eq!(competing.commits.get(), 2);
    assert_eq!(source.request_count(), 4);
    assert_eq!(revisions(&harness.week(MONDAY)), [1, 1, 0, 0, 0]);
}

#[test]
fn current_conflict_refreshes_the_clock_and_publishes_the_new_current_day() {
    let wednesday = day(2);
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([
        (MONDAY, hours(7.0)),
        (tuesday(), hours(7.0)),
        (wednesday, hours(7.0)),
    ]);
    let competing =
        CompetingCommitStore::new(harness.store(), || harness.clock.set(at(wednesday, 18)));

    let current = harness
        .current_with(&source, &competing, &Settings::default(), &utc_zones())
        .unwrap();

    assert_eq!(current.workday().date, wednesday);
    assert_eq!(competing.reads.get(), 2);
    assert_eq!(competing.commits.get(), 2);
    assert_eq!(source.request_count(), 5);
    assert_eq!(revisions(&harness.week(MONDAY)), [1, 2, 1, 0, 0]);
}

#[test]
fn ledger_insert_failure_rolls_back_and_keeps_the_published_summary() {
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([(MONDAY, hours(8.0)), (tuesday(), hours(6.0))]);
    let mut coordinator = RefreshCoordinator::new();
    let published = coordinator
        .refresh(|_| harness.current(&source))
        .unwrap()
        .clone();
    let before = harness.week(MONDAY);
    harness.execute(
        "CREATE TRIGGER FailWeeklyRuntimeLedger BEFORE INSERT ON CreditLedgerEntry
         BEGIN SELECT RAISE(ABORT, 'injected weekly runtime ledger failure'); END;",
    );
    source.work.borrow_mut().insert(MONDAY, hours(8.5));

    let error = coordinator
        .refresh(|_| harness.current(&source))
        .unwrap_err();

    assert!(
        matches!(
            error,
            WeeklyError::Store(WeeklyCreditError::Storage(StorageError::Sqlite(_)))
        ),
        "{error:?}"
    );
    let after = harness.week(MONDAY);
    assert_eq!(revisions(&after), revisions(&before));
    assert_eq!(after.ledger(), before.ledger());
    assert_eq!(coordinator.current(), Some(&published));
}

#[test]
fn repeated_commit_conflicts_stop_after_three_complete_attempts() {
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([(MONDAY, hours(7.0)), (tuesday(), hours(7.0))]);
    let empty = Database::new();
    let stale = AlwaysStaleStore {
        empty: WeeklyCreditRepository::new(&empty.connection),
        reads: Cell::new(0),
        commits: Cell::new(0),
    };

    let error = harness
        .current_with(&source, &stale, &Settings::default(), &utc_zones())
        .unwrap_err();

    assert!(
        matches!(error, WeeklyError::Store(WeeklyCreditError::Stale { .. })),
        "{error:?}"
    );
    assert_eq!(stale.reads.get(), 3);
    assert_eq!(stale.commits.get(), 3);
    assert_eq!(source.request_count(), 6);
}

#[test]
fn clock_rollback_rejects_later_and_future_heads_before_source_reads() {
    let harness = Harness::new(at(tuesday(), 18));
    let source = WorkSource::new([(MONDAY, hours(7.0)), (tuesday(), hours(7.0))]);
    harness.current(&source).unwrap();
    let calls_after_seed = source.request_count();

    harness.clock.set(at(MONDAY, 19));
    let later = harness.current(&source).unwrap_err();
    assert!(
        matches!(later, WeeklyError::ClockRollbackDropsWorkday(date) if date == tuesday()),
        "{later:?}"
    );
    assert_eq!(
        later.to_string(),
        "Clock rollback would drop persisted workday 2026-09-08."
    );
    assert_eq!(source.request_count(), calls_after_seed);

    harness.clock.set(at(tuesday(), 17));
    let future = harness.current(&source).unwrap_err();
    assert!(
        matches!(future, WeeklyError::ClockRollbackPrecedesMaterial(date) if date == MONDAY),
        "{future:?}"
    );
    assert_eq!(source.request_count(), calls_after_seed);
}

#[test]
fn saturday_rollback_before_persisted_friday_closure_rejects_before_source_reads_or_writes() {
    let saturday = day(5);
    let harness = Harness::new(at(saturday, 11));
    let source = WorkSource::new(workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]));
    harness.current(&source).unwrap();
    let persisted_closure = harness.week(MONDAY);
    let calls_after_closure = source.request_count();

    harness.clock.set(at(saturday, 10));
    let error = harness.current(&source).unwrap_err();

    assert!(
        matches!(error, WeeklyError::ClockRollbackPrecedesMaterial(date) if date == MONDAY),
        "{error:?}"
    );
    let after_rollback = harness.week(MONDAY);
    assert_eq!(source.request_count(), calls_after_closure);
    assert_eq!(revisions(&after_rollback), revisions(&persisted_closure));
    assert_eq!(after_rollback.ledger(), persisted_closure.ledger());
}

#[test]
fn friday_expiry_changes_once_and_following_monday_starts_at_zero() {
    let friday = day(4);
    let saturday = day(5);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);

    harness.current(&source).unwrap();
    let before_expiry = harness.week(MONDAY);
    assert_eq!(
        before_expiry
            .latest_revisions()
            .last()
            .unwrap()
            .snapshot()
            .carry_available(),
        hours(1.0)
    );
    assert!(expiries(&before_expiry).is_empty());

    harness.clock.set(at(saturday, 10));
    harness.current(&source).unwrap();
    let expired = harness.week(MONDAY);
    let friday_revision = head(&expired, friday);
    let expiry = single_expiry(&expired);
    assert_eq!(
        friday_revision.snapshot().carry_available(),
        SignedDuration::ZERO
    );
    assert_eq!(friday_revision.snapshot().carry_expired(), hours(1.0));
    assert_eq!(expiry.origin().revision, friday_revision.revision());

    harness.clock.set(at(saturday, 11));
    harness.current(&source).unwrap();
    let repeated = harness.week(MONDAY);
    assert_eq!(
        repeated.revision_vector().revision(friday).unwrap(),
        friday_revision.revision()
    );
    assert_eq!(single_expiry(&repeated).created_at(), expiry.created_at());

    harness.clock.set(at(following_monday, 18));
    let monday_summary = harness.current(&source).unwrap();
    assert_eq!(monday_summary.applied_carry_credit(), SignedDuration::ZERO);
    assert!(harness.week(following_monday).ledger().is_empty());
}

#[test]
fn following_monday_closes_an_unexpired_complete_week_and_restart_keeps_its_identity() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();

    harness.clock.set(at(following_monday, 18));
    let current = harness.current(&source).unwrap();
    let closed = harness.week(MONDAY);
    let expiry = single_expiry(&closed);

    assert_eq!(current.applied_carry_credit(), SignedDuration::ZERO);
    assert_eq!(revisions(&closed), [1, 1, 1, 1, 2]);
    assert_eq!(expiry.amount(), hours(1.0));
    assert_eq!(expiry.workday_date(), friday);
    assert_eq!(
        expiry.origin(),
        CreditOrigin {
            workday_date: friday,
            revision: 2
        }
    );
    assert_eq!(expiry.created_at(), harness.clock.now());

    harness.current(&source).unwrap();
    let repeated = harness.week(MONDAY);
    let repeated_expiry = single_expiry(&repeated);
    assert_eq!(revisions(&repeated), revisions(&closed));
    assert_eq!(repeated_expiry.id(), expiry.id());
    assert_eq!(repeated_expiry.created_at(), expiry.created_at());
}

#[test]
fn partial_old_week_defers_on_stale_missing_friday_while_current_week_publishes_then_recovers() {
    let thursday = day(3);
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(thursday, 18));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();
    let before = harness.week(MONDAY);
    source.stale.borrow_mut().insert(friday);

    harness.clock.set(at(following_monday, 18));
    let current = harness.current(&source).unwrap();
    let deferred = harness.week(MONDAY);

    assert_eq!(current.applied_carry_credit(), SignedDuration::ZERO);
    assert_eq!(revisions(&deferred), revisions(&before));
    assert_eq!(deferred.ledger(), before.ledger());
    assert_eq!(deferred.revision_vector().revision(friday), Some(0));
    assert_eq!(revisions(&harness.week(following_monday)), [1, 0, 0, 0, 0]);

    source.stale.borrow_mut().clear();
    harness.current(&source).unwrap();
    let recovered = harness.week(MONDAY);
    let friday_head = head(&recovered, friday).snapshot();

    assert_eq!(revisions(&recovered), [1, 1, 1, 2, 1]);
    assert_eq!(friday_head.credited_work(), hours(7.0));
    assert!(friday_head.unknown_work().is_positive());
    assert_eq!(friday_head.source_quality(), SourceQuality::Complete);
    assert_eq!(single_expiry(&recovered).amount(), hours(1.0));
}

#[test]
fn fresh_empty_historical_snapshot_defers_without_erasing_credit_and_current_week_still_commits() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();
    let before = harness.week(MONDAY);

    source
        .work
        .borrow_mut()
        .insert(MONDAY, SignedDuration::ZERO);
    harness.clock.set(at(following_monday, 18));
    let current = harness.current(&source).unwrap();
    let deferred = harness.week(MONDAY);

    assert_eq!(revisions(&deferred), revisions(&before));
    assert_eq!(deferred.ledger(), before.ledger());
    assert_eq!(current.applied_carry_credit(), SignedDuration::ZERO);
    assert_eq!(revisions(&harness.week(following_monday)), [1, 0, 0, 0, 0]);
}

#[test]
fn unavailable_oldest_week_does_not_block_another_old_week_or_the_current_week() {
    let second_monday = day(7);
    let current_monday = day(14);
    let harness = Harness::new(at(day(4), 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.extend(workweek(second_monday, [8.0, 7.0, 7.0, 7.0, 7.0]));
    work.push((current_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();

    source.unavailable.borrow_mut().insert(day(2));
    harness.clock.set(at(day(11), 20));
    harness.current(&source).unwrap();
    let first_before = harness.week(MONDAY);

    let requests_before = source.request_count();
    harness.clock.set(at(current_monday, 18));
    let current = harness.current(&source).unwrap();
    let requested_dates: Vec<Date> = source.requests.borrow()[requests_before..]
        .iter()
        .map(|request| start_date(*request))
        .collect();
    let first_deferred = harness.week(MONDAY);
    let second_closed = harness.week(second_monday);

    assert_eq!(revisions(&first_deferred), revisions(&first_before));
    assert!(expiries(&first_deferred).is_empty());
    assert_eq!(expiries(&second_closed).len(), 1);
    assert_eq!(current.applied_carry_credit(), SignedDuration::ZERO);
    assert_eq!(requested_dates.last(), Some(&current_monday));
    let position = |date: Date| {
        requested_dates
            .iter()
            .position(|requested| *requested == date)
    };
    assert!(position(MONDAY).unwrap() < position(second_monday).unwrap());
}

#[test]
fn complete_replay_uses_changed_settings_and_the_persisted_timezone() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    harness
        .current(&WorkSource::new(work.clone()).complete())
        .unwrap();

    let changed = Settings {
        nominal_daily_target: hours(8.0),
        minimum_effective_target: hours(6.0),
        ..Settings::default()
    };
    let changed_source = WorkSource::new(work).complete().utc_or_paris();
    harness.clock.set(at(following_monday, 18));
    harness
        .current_with(
            &changed_source,
            &harness.store(),
            &changed,
            &CatalogZones::paris_with_utc(),
        )
        .unwrap();
    let replayed = harness.week(MONDAY);

    assert_eq!(revisions(&replayed), [2, 2, 2, 2, 2]);
    assert!(replayed.latest_revisions().iter().all(|revision| {
        revision.snapshot().workday().time_zone_id == "UTC"
            && revision.snapshot().nominal_target() == hours(8.0)
    }));
    assert!(replayed.ledger().is_empty());
    assert_eq!(
        replayed
            .latest_revisions()
            .last()
            .unwrap()
            .snapshot()
            .carry_available(),
        SignedDuration::ZERO
    );
}

#[test]
fn historical_replay_uses_the_persisted_timezone_and_retains_its_manual_adjustment() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [7.0; 5]);
    work.push((following_monday, hours(7.0)));
    ManualAdjustmentRepository::new(&harness.database.connection)
        .append(
            &ManualAdjustment::duration_work(
                AdjustmentId::new("monday-hour").unwrap(),
                LocalWorkday::new(MONDAY, "UTC"),
                hours(1.0),
                at(MONDAY, 19),
            )
            .unwrap()
            .with_reason("complete Monday"),
        )
        .unwrap();
    harness
        .current(&WorkSource::new(work.clone()).complete())
        .unwrap();
    assert_eq!(
        harness.week(MONDAY).latest_revisions()[0]
            .snapshot()
            .credited_work(),
        hours(8.0)
    );

    let changed_source = WorkSource::new(work).complete().utc_or_paris();
    harness.clock.set(at(following_monday, 18));
    harness
        .current_with(
            &changed_source,
            &harness.store(),
            &Settings::default(),
            &CatalogZones::paris_with_utc(),
        )
        .unwrap();
    let replayed = harness.week(MONDAY);
    let monday_head = replayed.latest_revisions()[0].snapshot();

    assert_eq!(monday_head.workday().time_zone_id, "UTC");
    assert_eq!(monday_head.credited_work(), hours(8.0));
    assert!(
        monday_head
            .breakdown()
            .entries()
            .iter()
            .any(|entry| entry.adjustment_id().map(AdjustmentId::as_str) == Some("monday-hour"))
    );
    assert_eq!(expiries(&replayed).len(), 1);
}

#[test]
fn mixed_persisted_timezones_reject_historical_replay_before_source_reads() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();
    harness.execute(
        "UPDATE DailySummaryRevision SET TimezoneId = 'Europe/Paris' \
         WHERE WorkdayDate = '2026-09-09';",
    );
    let source_reads_before_replay = source.request_count();

    harness.clock.set(at(following_monday, 18));
    let error = harness.current(&source).unwrap_err();

    assert!(
        matches!(error, WeeklyError::MixedTimeZones(monday) if monday == MONDAY),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "Historical week 2026-09-07 must have one persisted timezone."
    );
    assert_eq!(source.request_count(), source_reads_before_replay);
    assert_eq!(revisions(&harness.week(following_monday)), [0; 5]);
}

#[test]
fn historical_closure_retries_three_coherent_reads_before_committing_current_week() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();
    let stale = HistoricalStaleStore {
        inner: harness.store(),
        historical_week: MONDAY,
        stale_attempts: 2,
        reads: Cell::new(0),
        commits: Cell::new(0),
    };

    harness.clock.set(at(following_monday, 18));
    harness
        .current_with(&source, &stale, &Settings::default(), &utc_zones())
        .unwrap();

    assert_eq!(stale.reads.get(), 3);
    assert_eq!(stale.commits.get(), 3);
    assert_eq!(expiries(&harness.week(MONDAY)).len(), 1);
    assert_eq!(revisions(&harness.week(following_monday)), [1, 0, 0, 0, 0]);
}

#[test]
fn historical_expiry_insert_failure_rolls_back_the_old_week_and_skips_current_commit() {
    let friday = day(4);
    let following_monday = day(7);
    let harness = Harness::new(at(friday, 20));
    let mut work = workweek(MONDAY, [8.0, 7.0, 7.0, 7.0, 7.0]);
    work.push((following_monday, hours(7.0)));
    let source = WorkSource::new(work);
    harness.current(&source).unwrap();
    let before = harness.week(MONDAY);
    harness.execute(
        "CREATE TRIGGER FailHistoricalExpiry BEFORE INSERT ON CreditLedgerEntry
         WHEN NEW.Kind = 'expired'
         BEGIN SELECT RAISE(ABORT, 'injected historical expiry failure'); END;",
    );

    harness.clock.set(at(following_monday, 18));
    let error = harness.current(&source).unwrap_err();

    assert!(
        matches!(
            error,
            WeeklyError::Store(WeeklyCreditError::Storage(StorageError::Sqlite(_)))
        ),
        "{error:?}"
    );
    let after = harness.week(MONDAY);
    assert_eq!(revisions(&after), revisions(&before));
    assert_eq!(after.ledger(), before.ledger());
    assert_eq!(revisions(&harness.week(following_monday)), [0; 5]);
}
