//! Ported from the correction cases of `TrackingControlIntegrationTests.cs`,
//! `SuspensionCorrectionIntegrationTests.cs` and the non-demo cases of
//! `HistoryIntegrationTests.cs`, against a migrated SQLite ledger. The .NET accounting that read
//! an empty in-memory event store reads the empty ledger here.

use std::cell::RefCell;
use std::collections::HashMap;

use bientot_domain::accounting::{
    ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin, LongLockReviewId,
    OperationKey, PowerState, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::adjustments::ManualClassification;
use bientot_domain::credit::CreditLedgerEntryKind;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::SourceHealth;
use bientot_domain::time::{Clock, FrozenClock};
use bientot_store::activity::ActivityError;
use bientot_store::adjustments::AppendAdjustmentOutcome;
use bientot_store::credit::{WeeklyCreditRepository, WeeklyCreditState};
use bientot_store::history::DailySummaryRepository;
use bientot_store::sessions::{RecordTransitionOutcome, SessionEventRepository};
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp};
use rusqlite::Connection;

use super::*;
use crate::history::HistoryService;
use crate::sources::{ActivityReadResult, ActivitySource};
use crate::test_support::{Database, FullyActive, count, utc_accounting, utc_zones};
use crate::tracking::TrackingControl;
use crate::weekly::WeeklyRecalculation;

fn on(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn days_after(day: Date, days: i64) -> Date {
    day.checked_add(Span::new().days(days)).unwrap()
}

fn hours(value: f64) -> SignedDuration {
    SignedDuration::from_secs_f64(value * 3600.0)
}

fn id(value: &str) -> AdjustmentId {
    AdjustmentId::new(value).unwrap()
}

fn utc_day(day: Date) -> LocalWorkday {
    LocalWorkday::new(day, "UTC")
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn duration_work(
    value: &str,
    day: Date,
    duration: SignedDuration,
    created_at: Timestamp,
) -> ManualAdjustment {
    ManualAdjustment::duration_work(id(value), utc_day(day), duration, created_at).unwrap()
}

fn recorded(value: &str) -> AppendAdjustmentResult {
    AppendAdjustmentResult {
        outcome: AppendAdjustmentOutcome::AlreadyRecorded,
        authoritative_id: id(value),
    }
}

fn inserted(value: &str) -> AppendAdjustmentResult {
    AppendAdjustmentResult {
        outcome: AppendAdjustmentOutcome::Inserted,
        authoritative_id: id(value),
    }
}

fn stored(connection: &Connection, workday: &LocalWorkday) -> Vec<ManualAdjustment> {
    ManualAdjustmentRepository::new(connection)
        .read_for_workday(workday)
        .unwrap()
}

fn record(connection: &Connection, key: &str, at: Timestamp, kind: WorkdayTransitionKind) {
    let clock = FrozenClock::new(at);
    SessionEventRepository::new(connection, &clock)
        .record(
            OperationKey::new(key).unwrap(),
            WorkdayTransition::new(at, "UTC", kind).unwrap(),
        )
        .unwrap();
}

fn assert_correction_refused(
    result: Result<AppendAdjustmentResult, CorrectionError>,
    expected: AdjustmentValidationError,
) {
    match result {
        Err(CorrectionError::Adjustments(ManualAdjustmentError::Validation(error))) => {
            assert_eq!(error, expected);
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

/// The calendar of `TrackingControlIntegrationTests.cs`: Monday 2026-08-24, active throughout.
const TRACKED_MONDAY: Date = date(2026, 8, 24);

fn tracked_at(hour: i8) -> Timestamp {
    on(TRACKED_MONDAY, hour, 0)
}

fn monday_credited(
    connection: &Connection,
    clock: &FrozenClock,
) -> (SignedDuration, Vec<AdjustmentId>) {
    let zones = utc_zones();
    let summary = utc_accounting(connection, &FullyActive, clock, &zones)
        .calculate_current()
        .unwrap();
    (
        summary.credited_work(),
        summary
            .accounting()
            .manual_duration_credits()
            .iter()
            .map(|credit| credit.adjustment_id().clone())
            .collect(),
    )
}

#[test]
fn pause_resume_and_adjustment_survive_reopen_and_exact_retries_contribute_once() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(9));
    let zones = utc_zones();
    let pause_key = OperationKey::new("tray-pause-20260824-0900").unwrap();
    let resume_key = OperationKey::new("tray-resume-20260824-1000").unwrap();
    let adjustment = duration_work(
        "manual-duration-30",
        TRACKED_MONDAY,
        SignedDuration::from_mins(30),
        tracked_at(11),
    );
    let first = {
        let tracking = TrackingControl::new(
            SessionEventRepository::new(&database.connection, &clock),
            &clock,
            &zones,
            &|| {},
        );
        let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
        let corrections = ManualAdjustmentCommands::new(
            &accounting,
            ManualAdjustmentRepository::new(&database.connection),
        );
        let pause = tracking.pause(pause_key.clone()).unwrap();
        clock.set(tracked_at(10));
        let resume = tracking.resume(resume_key.clone()).unwrap();
        clock.set(tracked_at(11));
        let appended = corrections.save(&adjustment).unwrap();
        (pause, resume, appended)
    };
    let first_summary = monday_credited(&database.connection, &clock);

    let reopened = database.reopen();
    let tracking = TrackingControl::new(
        SessionEventRepository::new(&reopened, &clock),
        &clock,
        &zones,
        &|| {},
    );
    let accounting = utc_accounting(&reopened, &FullyActive, &clock, &zones);
    let corrections =
        ManualAdjustmentCommands::new(&accounting, ManualAdjustmentRepository::new(&reopened));
    clock.set(tracked_at(9));
    let pause_retry = tracking.pause(pause_key).unwrap();
    clock.set(tracked_at(10));
    let resume_retry = tracking.resume(resume_key).unwrap();
    clock.set(tracked_at(11));
    let adjustment_retry = corrections.save(&adjustment).unwrap();
    let reopened_summary = monday_credited(&reopened, &clock);

    let (pause, resume, appended) = first;
    assert_eq!(
        [pause.outcome, resume.outcome],
        [RecordTransitionOutcome::Inserted; 2]
    );
    assert_eq!(
        [pause_retry, resume_retry].map(|retry| (retry.outcome, retry.authoritative_id)),
        [
            (
                RecordTransitionOutcome::AlreadyRecorded,
                pause.authoritative_id
            ),
            (
                RecordTransitionOutcome::AlreadyRecorded,
                resume.authoritative_id
            ),
        ]
    );
    assert_eq!(appended, inserted("manual-duration-30"));
    assert_eq!(adjustment_retry, recorded("manual-duration-30"));
    assert_eq!(first_summary, (hours(10.5), vec![id("manual-duration-30")]));
    assert_eq!(reopened_summary, first_summary);
    assert_eq!(count(&reopened, "SessionEvent"), 2);
    assert_eq!(count(&reopened, "ManualAdjustment"), 1);
}

/// A retry of a correction and of a revocation, later the same day and the next day, finds each
/// already recorded.
#[test]
fn a_retry_after_time_advances_finds_the_correction_already_recorded() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(11));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let adjustment = duration_work(
        "duration-at-eleven",
        TRACKED_MONDAY,
        SignedDuration::from_mins(30),
        tracked_at(11),
    );
    let revocation = ManualAdjustment::revocation(
        id("revoke-at-eleven"),
        utc_day(TRACKED_MONDAY),
        id("duration-at-eleven"),
        tracked_at(11) + SignedDuration::from_mins(1),
    )
    .unwrap();
    assert_eq!(
        corrections.save(&adjustment).unwrap(),
        inserted("duration-at-eleven")
    );
    assert_eq!(
        corrections.save(&revocation).unwrap(),
        inserted("revoke-at-eleven")
    );

    for later in [tracked_at(15), on(date(2026, 8, 25), 9, 0)] {
        clock.set(later);
        assert_eq!(
            corrections.save(&adjustment).unwrap(),
            recorded("duration-at-eleven")
        );
        assert_eq!(
            corrections.save(&revocation).unwrap(),
            recorded("revoke-at-eleven")
        );
    }
    assert_eq!(
        stored(&database.connection, &utc_day(TRACKED_MONDAY)),
        [adjustment, revocation]
    );
}

#[test]
fn overlapping_manual_work_is_rejected_before_persistence_and_remains_absent_after_reopen() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(11));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let overlapping = ManualAdjustment::interval_work(
        id("overlapping-manual-work"),
        utc_day(TRACKED_MONDAY),
        interval(tracked_at(9), tracked_at(10)),
        tracked_at(11),
    )
    .unwrap();

    assert_correction_refused(
        corrections.save(&overlapping),
        AdjustmentValidationError::OverlapsCreditedCoverage {
            id: id("overlapping-manual-work"),
        },
    );

    assert_eq!(count(&database.connection, "ManualAdjustment"), 0);
    assert_eq!(stored(&database.reopen(), &utc_day(TRACKED_MONDAY)), []);
}

#[test]
fn editing_latest_duration_supersedes_it_and_counts_only_replacement_after_reopen() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(11));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let original = duration_work(
        "duration-original",
        TRACKED_MONDAY,
        SignedDuration::from_mins(30),
        tracked_at(11),
    );
    corrections.save(&original).unwrap();

    let selected = corrections
        .latest_active(&utc_day(TRACKED_MONDAY))
        .unwrap()
        .unwrap();
    let replacement = duration_work(
        "duration-replacement",
        TRACKED_MONDAY,
        SignedDuration::from_mins(45),
        tracked_at(11) + SignedDuration::from_mins(1),
    )
    .superseding(selected.id().clone());
    corrections.save(&replacement).unwrap();

    let reopened = database.reopen();
    let reopened_accounting = utc_accounting(&reopened, &FullyActive, &clock, &zones);
    let reopened_corrections = ManualAdjustmentCommands::new(
        &reopened_accounting,
        ManualAdjustmentRepository::new(&reopened),
    );
    let summary = reopened_accounting.calculate_current().unwrap();
    let credits: Vec<_> = summary
        .accounting()
        .manual_duration_credits()
        .iter()
        .map(|credit| (credit.adjustment_id().clone(), credit.duration()))
        .collect();
    assert_eq!(
        credits,
        [(id("duration-replacement"), SignedDuration::from_mins(45))]
    );
    assert_eq!(summary.credited_work(), hours(11.75));
    assert_eq!(
        reopened_corrections
            .latest_active(&utc_day(TRACKED_MONDAY))
            .unwrap(),
        Some(replacement)
    );
    assert_eq!(count(&reopened, "ManualAdjustment"), 2);
}

#[test]
fn revoking_the_latest_duration_removes_its_credit_and_leaves_the_earlier_one_to_edit() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(11));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let earlier = duration_work(
        "duration-earlier",
        TRACKED_MONDAY,
        SignedDuration::from_mins(30),
        tracked_at(11),
    );
    let later = duration_work(
        "duration-later",
        TRACKED_MONDAY,
        SignedDuration::from_mins(15),
        tracked_at(11) + SignedDuration::from_mins(1),
    );
    corrections.save(&earlier).unwrap();
    corrections.save(&later).unwrap();

    corrections
        .save(
            &ManualAdjustment::revocation(
                id("revoke-later"),
                utc_day(TRACKED_MONDAY),
                id("duration-later"),
                tracked_at(11) + SignedDuration::from_mins(2),
            )
            .unwrap(),
        )
        .unwrap();

    let reopened = database.reopen();
    assert_eq!(
        monday_credited(&reopened, &clock),
        (hours(11.5), vec![id("duration-earlier")])
    );
    let reopened_accounting = utc_accounting(&reopened, &FullyActive, &clock, &zones);
    let reopened_corrections = ManualAdjustmentCommands::new(
        &reopened_accounting,
        ManualAdjustmentRepository::new(&reopened),
    );
    assert_eq!(
        reopened_corrections
            .latest_active(&utc_day(TRACKED_MONDAY))
            .unwrap(),
        Some(earlier)
    );
    assert_eq!(count(&reopened, "ManualAdjustment"), 3);
}

/// Not in .NET: two adjustments created at the same instant are ordered by ordinal id.
#[test]
fn latest_active_breaks_a_creation_tie_by_the_greater_id() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(11));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    for value in ["duration-b", "duration-a"] {
        corrections
            .save(&duration_work(
                value,
                TRACKED_MONDAY,
                SignedDuration::from_mins(10),
                tracked_at(11),
            ))
            .unwrap();
    }

    let latest = corrections
        .latest_active(&utc_day(TRACKED_MONDAY))
        .unwrap()
        .unwrap();

    assert_eq!(latest.id(), &id("duration-b"));
}

#[test]
fn an_edited_long_lock_resolution_cannot_be_revoked() {
    let database = Database::new();
    let clock = FrozenClock::new(tracked_at(11));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let monday = utc_day(TRACKED_MONDAY);
    let resolution = ManualAdjustment::reclassification(
        id("long-lock-review:2026-08-24:UTC:session-lock:42:exclude"),
        monday.clone(),
        interval(tracked_at(9), tracked_at(10)),
        ManualClassification::Excluded,
        tracked_at(11),
    )
    .unwrap();
    let edit = ManualAdjustment::reclassification(
        id("edit-1"),
        monday.clone(),
        interval(tracked_at(9), tracked_at(10)),
        ManualClassification::Excluded,
        tracked_at(11) + SignedDuration::from_mins(1),
    )
    .unwrap()
    .superseding(resolution.id().clone())
    .with_reason("meeting");
    corrections.save(&resolution).unwrap();
    corrections.save(&edit).unwrap();
    let revoke = |revocation_id: &str, target: &ManualAdjustment| {
        corrections.save(
            &ManualAdjustment::revocation(
                id(revocation_id),
                monday.clone(),
                target.id().clone(),
                tracked_at(11) + SignedDuration::from_mins(2),
            )
            .unwrap(),
        )
    };

    assert_correction_refused(
        revoke("revoke-1", &resolution),
        AdjustmentValidationError::InvalidRevocationTarget {
            revocation_id: id("revoke-1"),
            target_id: resolution.id().clone(),
        },
    );
    assert_correction_refused(
        revoke("revoke-2", &edit),
        AdjustmentValidationError::InvalidRevocationTarget {
            revocation_id: id("revoke-2"),
            target_id: id("edit-1"),
        },
    );
    assert_eq!(corrections.latest_active(&monday).unwrap(), Some(edit));
    assert_eq!(count(&database.connection, "ManualAdjustment"), 2);
}

/// The week of `SuspensionCorrectionIntegrationTests.cs` and `HistoryIntegrationTests.cs`.
const MONDAY: Date = date(2026, 9, 7);

fn friday() -> Date {
    days_after(MONDAY, 4)
}

/// The .NET `WorkingWeekSource` and `CompleteActivitySource`: the day a read starts on is away
/// until 08:00, active for its hours, then away again, as a healthy source reports it.
struct OfficeDay {
    work: HashMap<Date, SignedDuration>,
    requests: RefCell<Vec<UtcInterval>>,
}

impl OfficeDay {
    fn new(work: impl IntoIterator<Item = (Date, SignedDuration)>) -> Self {
        Self {
            work: work.into_iter().collect(),
            requests: RefCell::default(),
        }
    }

    /// `hours` on each of the `days` days from [`MONDAY`].
    fn uniform(days: i64, hours_worked: f64) -> Self {
        Self::new((0..days).map(|offset| (days_after(MONDAY, offset), hours(hours_worked))))
    }

    fn request_count(&self) -> usize {
        self.requests.borrow().len()
    }
}

impl ActivitySource for OfficeDay {
    fn read(&self, request: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        self.requests.borrow_mut().push(request);
        let day = request.start().to_zoned(TimeZone::UTC).date();
        let active_start = request.start() + SignedDuration::from_hours(8);
        let active_end = active_start + self.work[&day];
        let origin = || AutomaticOrigin::new("activitywatch:test").unwrap();
        Ok(ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(
                request,
                [
                    (request.start(), active_start, ActivityState::Afk),
                    (active_start, active_end, ActivityState::Active),
                    (active_end, request.end(), ActivityState::Afk),
                ]
                .into_iter()
                .filter(|(start, end, _)| start < end)
                .map(|(start, end, state)| {
                    ActivitySegment::new(interval(start, end), state, origin())
                }),
            ),
            health: SourceHealth::healthy("activitywatch:test", request.end()),
        })
    }
}

fn week_credited(state: &WeeklyCreditState) -> Vec<SignedDuration> {
    state
        .latest_revisions()
        .iter()
        .map(|revision| revision.snapshot().credited_work())
        .collect()
}

fn total(durations: &[SignedDuration]) -> SignedDuration {
    durations.iter().copied().sum()
}

#[test]
fn current_week_suspension_correction_edit_and_reopen_preserve_weekly_totals_and_audit_history() {
    let database = Database::new();
    let clock = FrozenClock::new(on(friday(), 18, 0));
    let zones = utc_zones();
    let source = OfficeDay::uniform(5, 7.0);
    record(
        &database.connection,
        "monday-suspend",
        on(MONDAY, 9, 0),
        WorkdayTransitionKind::PowerObserved(PowerState::Suspended),
    );
    record(
        &database.connection,
        "monday-resume",
        on(MONDAY, 10, 0),
        WorkdayTransitionKind::PowerObserved(PowerState::Active),
    );
    let accounting = utc_accounting(&database.connection, &source, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let store = WeeklyCreditRepository::new(&database.connection);
    let weekly = WeeklyRecalculation::new(&accounting, &store);
    let workday = utc_day(MONDAY);
    let original = ManualAdjustment::interval_work(
        id("suspension-work"),
        workday.clone(),
        interval(on(MONDAY, 9, 0), on(MONDAY, 10, 0)),
        on(friday(), 17, 0),
    )
    .unwrap()
    .with_reason("client workshop");
    let correction = ManualAdjustment::reclassification(
        id("suspension-work-edit"),
        workday.clone(),
        interval(on(MONDAY, 9, 30), on(MONDAY, 10, 0)),
        ManualClassification::Credited,
        on(friday(), 17, 1),
    )
    .unwrap()
    .superseding(original.id().clone())
    .with_reason("workshop started later");

    corrections.save(&original).unwrap();
    weekly.calculate_current().unwrap();
    let original_week = week_credited(&store.read_week(MONDAY).unwrap());
    assert_eq!(original_week[0], hours(7.0));
    assert_eq!(total(&original_week), hours(35.0));

    corrections.save(&correction).unwrap();
    weekly.calculate_current().unwrap();
    let saved_week = week_credited(&store.read_week(MONDAY).unwrap());
    assert_eq!(saved_week[0], hours(6.5));
    assert_eq!(total(&saved_week), hours(34.5));

    let reopened = database.reopen();
    let reopened_accounting = utc_accounting(&reopened, &source, &clock, &zones);
    let reopened_store = WeeklyCreditRepository::new(&reopened);
    WeeklyRecalculation::new(&reopened_accounting, &reopened_store)
        .calculate_current()
        .unwrap();
    let reopened_week = week_credited(&reopened_store.read_week(MONDAY).unwrap());
    assert_eq!(reopened_week[0], hours(6.5));
    assert_eq!(total(&reopened_week), hours(34.5));
    assert_eq!(stored(&reopened, &workday), [original, correction]);
}

/// The .NET `FragmentedMeetingSource`: Monday morning split by unknown and away fragments around
/// a suspension.
struct FragmentedMeeting;

impl ActivitySource for FragmentedMeeting {
    fn read(&self, request: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        let origin = || AutomaticOrigin::new("demo:meeting").unwrap();
        let at = |hour, minute| on(MONDAY, hour, minute);
        let segments = [
            (request.start(), at(8, 0), ActivityState::Afk),
            (at(8, 0), at(9, 0), ActivityState::Active),
            (at(9, 0), at(9, 15), ActivityState::Unknown),
            (at(9, 15), at(9, 30), ActivityState::Afk),
            (at(9, 30), at(15, 0), ActivityState::Active),
            (at(15, 0), request.end(), ActivityState::Afk),
        ]
        .map(|(start, end, state)| ActivitySegment::new(interval(start, end), state, origin()));
        Ok(ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(request, segments),
            health: SourceHealth::healthy("demo", request.end()),
        })
    }
}

#[test]
fn one_meeting_crosses_automatic_fragments_without_double_counting_and_survives_reopen() {
    let database = Database::new();
    let clock = FrozenClock::new(on(MONDAY, 18, 0));
    let zones = utc_zones();
    record(
        &database.connection,
        "meeting-suspend",
        on(MONDAY, 9, 30),
        WorkdayTransitionKind::PowerObserved(PowerState::Suspended),
    );
    record(
        &database.connection,
        "meeting-resume",
        on(MONDAY, 9, 45),
        WorkdayTransitionKind::PowerObserved(PowerState::Active),
    );
    let accounting = utc_accounting(&database.connection, &FragmentedMeeting, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let workday = utc_day(MONDAY);
    let credited = |accounting: &WorkdayAccounting| {
        accounting
            .calculate(MONDAY, &accounting.capture_context())
            .unwrap()
            .summary_input
            .accounting()
            .credited_work()
    };
    assert_eq!(credited(&accounting), hours(6.25));
    let meeting = ManualAdjustment::reclassification(
        id("meeting"),
        workday.clone(),
        interval(on(MONDAY, 8, 45), on(MONDAY, 10, 0)),
        ManualClassification::Credited,
        on(MONDAY, 17, 0),
    )
    .unwrap()
    .with_reason("Meeting");

    corrections.save(&meeting).unwrap();
    corrections.save(&meeting).unwrap();
    assert_eq!(credited(&accounting), hours(7.0));
    assert_eq!(
        stored(&database.connection, &workday),
        std::slice::from_ref(&meeting)
    );
    let conflict = ManualAdjustment::exclusion(
        id("conflict"),
        workday.clone(),
        interval(on(MONDAY, 9, 0), on(MONDAY, 9, 15)),
        on(MONDAY, 17, 1),
    )
    .unwrap();
    assert_correction_refused(
        corrections.save(&conflict),
        AdjustmentValidationError::OverlappingActiveAdjustments,
    );

    let reopened = database.reopen();
    let reopened_accounting = utc_accounting(&reopened, &FragmentedMeeting, &clock, &zones);
    assert_eq!(credited(&reopened_accounting), hours(7.0));
    assert_eq!(stored(&reopened, &workday), [meeting]);
}

#[test]
fn exact_active_lookup_rejects_superseded_and_wrong_day_ids() {
    let database = Database::new();
    let clock = FrozenClock::new(on(friday(), 18, 0));
    let zones = utc_zones();
    let store = ManualAdjustmentRepository::new(&database.connection);
    let original = duration_work(
        "original",
        MONDAY,
        SignedDuration::from_mins(20),
        on(MONDAY, 10, 0),
    );
    let successor = duration_work(
        "successor",
        MONDAY,
        SignedDuration::from_mins(25),
        on(MONDAY, 11, 0),
    )
    .superseding(original.id().clone());
    store.append(&original).unwrap();
    store.append(&successor).unwrap();
    let source = OfficeDay::uniform(1, 7.0);
    let accounting = utc_accounting(&database.connection, &source, &clock, &zones);
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );

    assert_eq!(
        corrections.active(&utc_day(MONDAY), original.id()).unwrap(),
        None
    );
    assert_eq!(
        corrections
            .active(&utc_day(MONDAY), successor.id())
            .unwrap(),
        Some(successor.clone())
    );
    assert_eq!(
        corrections
            .active(&utc_day(days_after(MONDAY, 1)), successor.id())
            .unwrap(),
        None
    );
}

/// Credited work, effective target, carry applied, earned and available of a stored day.
fn day_values(state: &WeeklyCreditState) -> Vec<[SignedDuration; 5]> {
    state
        .latest_revisions()
        .iter()
        .map(|revision| {
            let snapshot = revision.snapshot();
            [
                snapshot.credited_work(),
                snapshot.effective_target(),
                snapshot.carry_applied(),
                snapshot.carry_earned(),
                snapshot.carry_available(),
            ]
        })
        .collect()
}

fn ledger_identities(state: &WeeklyCreditState) -> Vec<(String, Timestamp)> {
    state
        .ledger()
        .iter()
        .map(|entry| (entry.id().to_string(), entry.created_at()))
        .collect()
}

fn ledger_count(state: &WeeklyCreditState, kind: CreditLedgerEntryKind) -> usize {
    state
        .ledger()
        .iter()
        .filter(|entry| entry.kind() == kind)
        .count()
}

#[test]
fn monday_adjustment_on_friday_recalculates_every_head_and_survives_reopen() {
    let database = Database::new();
    let clock = FrozenClock::new(on(friday(), 18, 0));
    let zones = utc_zones();
    let source = OfficeDay::uniform(5, 7.0);
    let accounting = utc_accounting(&database.connection, &source, &clock, &zones);
    let store = WeeklyCreditRepository::new(&database.connection);
    let weekly = WeeklyRecalculation::new(&accounting, &store);
    weekly.calculate_current().unwrap();
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let adjustment = duration_work(
        "monday-correction",
        MONDAY,
        SignedDuration::from_hours(1),
        on(friday(), 17, 0),
    )
    .with_reason("complete Monday");

    let saved = corrections.save(&adjustment).unwrap();
    weekly.calculate_current().unwrap();
    let corrected = store.read_week(MONDAY).unwrap();

    assert_eq!(saved, inserted("monday-correction"));
    assert_eq!(corrected.revision_vector().revisions(), [2; 5]);
    let (one, zero) = (hours(1.0), SignedDuration::ZERO);
    let following = [hours(7.0), hours(6.0), one, one, one];
    assert_eq!(
        day_values(&corrected),
        [
            [hours(8.0), hours(7.0), zero, one, one],
            following,
            following,
            following,
            following,
        ]
    );
    assert_eq!(
        [
            ledger_count(&corrected, CreditLedgerEntryKind::Earned),
            ledger_count(&corrected, CreditLedgerEntryKind::Applied),
            ledger_count(&corrected, CreditLedgerEntryKind::Expired),
        ],
        [5, 4, 0]
    );
    let movement_identities = ledger_identities(&corrected);

    let duplicate = corrections.save(&adjustment).unwrap();
    weekly.calculate_current().unwrap();
    let repeated = store.read_week(MONDAY).unwrap();
    assert_eq!(duplicate, recorded("monday-correction"));
    assert_eq!(repeated.revision_vector(), corrected.revision_vector());
    assert_eq!(ledger_identities(&repeated), movement_identities);

    let reopened = database.reopen();
    let reopened_week = HistoryService::new(DailySummaryRepository::new(&reopened), &clock, &zones)
        .current_week()
        .unwrap();
    let reopened_ledger = WeeklyCreditRepository::new(&reopened)
        .read_week(MONDAY)
        .unwrap();
    assert_eq!(reopened_week.week_start_date(), MONDAY);
    let heads: Vec<_> = reopened_week
        .days()
        .iter()
        .map(|day| {
            let revision = day.latest_revision.as_ref().unwrap();
            (
                revision.revision(),
                revision.snapshot().credited_work(),
                revision.snapshot().effective_target(),
            )
        })
        .collect();
    assert_eq!(
        heads,
        [
            (2, hours(8.0), hours(7.0)),
            (2, hours(7.0), hours(6.0)),
            (2, hours(7.0), hours(6.0)),
            (2, hours(7.0), hours(6.0)),
            (2, hours(7.0), hours(6.0)),
        ]
    );
    assert_eq!(ledger_identities(&reopened_ledger), movement_identities);
}

#[test]
fn past_overlap_wrong_timezone_and_future_date_are_rejected_before_persistence() {
    let database = Database::new();
    let clock = FrozenClock::new(on(friday(), 18, 0));
    let zones = utc_zones();
    let source = OfficeDay::uniform(5, 7.0);
    let accounting = utc_accounting(&database.connection, &source, &clock, &zones);
    let store = WeeklyCreditRepository::new(&database.connection);
    WeeklyRecalculation::new(&accounting, &store)
        .calculate_current()
        .unwrap();
    let before = store.read_week(MONDAY).unwrap();
    let corrections = ManualAdjustmentCommands::new(
        &accounting,
        ManualAdjustmentRepository::new(&database.connection),
    );
    let overlap = ManualAdjustment::interval_work(
        id("overlap"),
        utc_day(MONDAY),
        interval(on(MONDAY, 9, 0), on(MONDAY, 10, 0)),
        on(friday(), 17, 0),
    )
    .unwrap();
    let wrong_time_zone = ManualAdjustment::duration_work(
        id("wrong-zone"),
        LocalWorkday::new(MONDAY, "Europe/Paris"),
        SignedDuration::from_mins(30),
        on(friday(), 17, 0),
    )
    .unwrap();
    let future = duration_work(
        "future",
        days_after(MONDAY, 7),
        SignedDuration::from_mins(30),
        on(friday(), 17, 0),
    );

    assert_correction_refused(
        corrections.save(&overlap),
        AdjustmentValidationError::OverlapsCreditedCoverage { id: id("overlap") },
    );
    assert_correction_refused(
        corrections.save(&wrong_time_zone),
        AdjustmentValidationError::WrongWorkday {
            id: id("wrong-zone"),
        },
    );
    let reads_before_future = source.request_count();
    assert!(matches!(
        corrections.save(&future),
        Err(CorrectionError::Accounting(AccountingError::FutureWorkday))
    ));

    assert_eq!(source.request_count(), reads_before_future);
    assert_eq!(count(&database.connection, "ManualAdjustment"), 0);
    let after = store.read_week(MONDAY).unwrap();
    assert_eq!(after.revision_vector(), before.revision_vector());
    assert_eq!(after.ledger(), before.ledger());
}

#[test]
fn closed_prior_week_correction_is_rejected_without_changing_persisted_history() {
    for current_day_offset in [7, 13] {
        let database = Database::new();
        let clock = FrozenClock::new(on(friday(), 18, 0));
        let zones = utc_zones();
        let source = OfficeDay::new((0..14).map(|offset| {
            (
                days_after(MONDAY, offset),
                hours(if offset == 0 { 8.0 } else { 7.0 }),
            )
        }));
        let accounting = utc_accounting(&database.connection, &source, &clock, &zones);
        let store = WeeklyCreditRepository::new(&database.connection);
        let weekly = WeeklyRecalculation::new(&accounting, &store);
        weekly.calculate_current().unwrap();

        clock.set(on(days_after(MONDAY, current_day_offset), 18, 0));
        weekly.calculate_current().unwrap();
        let closed = store.read_week(MONDAY).unwrap();
        assert_eq!(ledger_count(&closed, CreditLedgerEntryKind::Expired), 1);
        assert_eq!(
            closed
                .latest_revisions()
                .last()
                .unwrap()
                .snapshot()
                .carry_available(),
            SignedDuration::ZERO
        );
        let corrections = ManualAdjustmentCommands::new(
            &accounting,
            ManualAdjustmentRepository::new(&database.connection),
        );
        let adjustment = duration_work(
            "closed-week-correction",
            MONDAY,
            SignedDuration::from_mins(30),
            clock.now(),
        );
        let reads_before_correction = source.request_count();

        assert!(matches!(
            corrections.save(&adjustment),
            Err(CorrectionError::OutsideCurrentWeek)
        ));

        assert_eq!(source.request_count(), reads_before_correction);
        assert_eq!(count(&database.connection, "ManualAdjustment"), 0);
        weekly.calculate_current().unwrap();
        let reopened = WeeklyCreditRepository::new(&database.reopen())
            .read_week(MONDAY)
            .unwrap();
        assert_eq!(reopened, closed);
    }
}

#[path = "revocable_tests.rs"]
mod revocable;
