//! A range over one office day, against a migrated SQLite ledger.

use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use worktime_domain::accounting::{
    ActivityCoverage, ActivitySegment, ActivityState, AutomaticOrigin, OperationKey, PowerState,
    SessionState, WorkdayTransition, WorkdayTransitionKind,
};
use worktime_domain::sources::SourceHealth;
use worktime_domain::time::{Clock, FrozenClock};
use worktime_store::activity::ActivityError;
use worktime_store::reviews::LongLockResolutionRepository;
use worktime_store::sessions::SessionEventRepository;

use super::*;
use crate::corrections::{CorrectionError, ManualAdjustmentCommands};
use crate::sources::{ActivityReadResult, ActivitySource};
use crate::test_support::{Database, count, utc_accounting, utc_zones};

const MONDAY: Date = date(2026, 8, 31);

fn at(hour: i8, minute: i8) -> Timestamp {
    MONDAY
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn interval(from: (i8, i8), to: (i8, i8)) -> UtcInterval {
    UtcInterval::new(at(from.0, from.1), at(to.0, to.1)).unwrap()
}

fn id(value: &str) -> AdjustmentId {
    AdjustmentId::new(value).unwrap()
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(MONDAY, "UTC")
}

/// Active 08:40–11:50, away for lunch until 12:30, active until 15:10, unknown until 15:25,
/// then active until 17:30.
struct OfficeDay;

impl ActivitySource for OfficeDay {
    fn read(&self, request: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        let origin = || AutomaticOrigin::new("activitywatch:test").unwrap();
        let segments = [
            (request.start(), at(8, 40), ActivityState::Afk),
            (at(8, 40), at(11, 50), ActivityState::Active),
            (at(11, 50), at(12, 30), ActivityState::Afk),
            (at(12, 30), at(15, 10), ActivityState::Active),
            (at(15, 10), at(15, 25), ActivityState::Unknown),
            (at(15, 25), at(17, 30), ActivityState::Active),
            (at(17, 30), request.end(), ActivityState::Afk),
        ]
        .into_iter()
        .filter_map(|(start, end, state)| {
            Some(ActivitySegment::new(
                UtcInterval::new(start, end).ok()?.intersect(request)?,
                state,
                origin(),
            ))
        });
        Ok(ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(request, segments),
            health: SourceHealth::healthy("activitywatch:test", request.end()),
        })
    }
}

struct Day {
    database: Database,
    clock: FrozenClock,
    zones: worktime_domain::time::FixedTimeZoneProvider,
}

impl Day {
    fn new() -> Self {
        Self {
            database: Database::new(),
            clock: FrozenClock::new(at(18, 0)),
            zones: utc_zones(),
        }
    }

    fn connection(&self) -> &Connection {
        &self.database.connection
    }

    fn with<T>(&self, run: impl FnOnce(&WorkdayAccounting, RangeCorrections) -> T) -> T {
        let connection = self.connection();
        let accounting = utc_accounting(connection, &OfficeDay, &self.clock, &self.zones);
        let ranges = RangeCorrections::new(
            &accounting,
            ManualAdjustmentRepository::new(connection),
            LongLockReviews::new(
                ManualAdjustmentRepository::new(connection),
                SessionEventRepository::new(connection, &self.clock),
                LongLockResolutionRepository::new(connection),
            ),
        );
        run(&accounting, ranges)
    }

    fn record(
        &self,
        from: (i8, i8),
        to: (i8, i8),
        classification: ManualClassification,
    ) -> Result<RangeOutcome, RangeError> {
        self.with(|_, ranges| {
            ranges.record(&RangeCommand {
                workday: workday(),
                range: interval(from, to),
                classification,
                id: id("range"),
                created_at: self.clock.now(),
            })
        })
    }

    fn credited(&self) -> SignedDuration {
        self.with(|accounting, _| {
            accounting
                .calculate(MONDAY, &accounting.capture_context())
                .unwrap()
                .summary_input
                .accounting()
                .credited_work()
        })
    }

    fn save(&self, adjustment: ManualAdjustment) {
        ManualAdjustmentRepository::new(self.connection())
            .append(&adjustment)
            .unwrap();
    }

    fn stored(&self) -> Vec<ManualAdjustment> {
        ManualAdjustmentRepository::new(self.connection())
            .read_for_workday(&workday())
            .unwrap()
    }

    fn lock(&self, from: (i8, i8), to: (i8, i8)) {
        self.observe(
            from,
            to,
            WorkdayTransitionKind::SessionObserved(SessionState::Locked),
            WorkdayTransitionKind::SessionObserved(SessionState::Unlocked),
        );
    }

    fn sleep(&self, from: (i8, i8), to: (i8, i8)) {
        self.observe(
            from,
            to,
            WorkdayTransitionKind::PowerObserved(PowerState::Suspended),
            WorkdayTransitionKind::PowerObserved(PowerState::Active),
        );
    }

    fn observe(
        &self,
        from: (i8, i8),
        to: (i8, i8),
        start: WorkdayTransitionKind,
        end: WorkdayTransitionKind,
    ) {
        for (key, instant, kind) in [
            ("start", at(from.0, from.1), start),
            ("end", at(to.0, to.1), end),
        ] {
            SessionEventRepository::new(self.connection(), &FrozenClock::new(instant))
                .record(
                    OperationKey::new(format!("{key}-{}-{}", from.0, from.1)).unwrap(),
                    WorkdayTransition::new(instant, "UTC", kind).unwrap(),
                )
                .unwrap();
        }
    }
}

const EXCLUDED: ManualClassification = ManualClassification::Excluded;
const CREDITED: ManualClassification = ManualClassification::Credited;

fn hours(value: f64) -> SignedDuration {
    SignedDuration::from_secs_f64(value * 3600.)
}

#[test]
fn excluding_lunch_and_the_chat_after_it_corrects_only_the_work() {
    let day = Day::new();
    let before = day.credited();

    let outcome = day.record((11, 50), (14, 0), EXCLUDED).unwrap();

    let exclusion = ManualAdjustment::exclusion(
        id("range.1"),
        workday(),
        interval((12, 30), (14, 0)),
        at(18, 0),
    )
    .unwrap();
    assert_eq!(day.stored(), [exclusion]);
    assert_eq!(day.credited(), before - hours(1.5));
    assert_eq!(outcome.undoable(), Some([id("range.1")].as_slice()));
}

#[test]
fn counting_an_afternoon_fills_only_its_gap() {
    let day = Day::new();
    let before = day.credited();

    let outcome = day.record((15, 0), (16, 0), CREDITED).unwrap();

    let work = ManualAdjustment::interval_work(
        id("range.1"),
        workday(),
        interval((15, 10), (15, 25)),
        at(18, 0),
    )
    .unwrap();
    assert_eq!(day.stored(), [work]);
    assert_eq!(day.credited(), before + SignedDuration::from_mins(15));
    assert!(outcome.undoable().is_some());
}

#[test]
fn a_range_trims_the_correction_it_overlaps_and_cannot_be_undone() {
    let day = Day::new();
    let original = ManualAdjustment::exclusion(
        id("lunch-chat"),
        workday(),
        interval((13, 0), (14, 0)),
        at(17, 0),
    )
    .unwrap()
    .with_reason("chat");
    day.save(original.clone());
    let before = day.credited();

    let outcome = day.record((13, 30), (15, 0), CREDITED).unwrap();

    let trimmed = ManualAdjustment::exclusion(
        id("range.1"),
        workday(),
        interval((13, 0), (13, 30)),
        at(18, 0),
    )
    .unwrap()
    .superseding(id("lunch-chat"))
    .with_reason("chat");
    // 13:30–14:00 was active: once trimmed, the day counts it without a correction.
    assert_eq!(day.stored(), [original, trimmed]);
    assert_eq!(day.credited(), before + SignedDuration::from_mins(30));
    assert_eq!(outcome.undoable(), None);
}

#[test]
fn a_trimmed_correction_leaves_a_correction_only_where_the_day_was_away() {
    let day = Day::new();
    day.save(
        ManualAdjustment::exclusion(
            id("late-morning"),
            workday(),
            interval((11, 0), (12, 30)),
            at(17, 0),
        )
        .unwrap(),
    );
    let before = day.credited();

    day.record((11, 30), (12, 30), CREDITED).unwrap();

    let kinds: Vec<_> = day
        .stored()
        .iter()
        .skip(1)
        .map(|adjustment| (adjustment.id().clone(), adjustment.kind().clone()))
        .collect();
    assert_eq!(
        kinds,
        [
            (
                id("range.1"),
                AdjustmentKind::Exclusion {
                    interval: interval((11, 0), (11, 30))
                }
            ),
            (
                id("range.2"),
                AdjustmentKind::IntervalWork {
                    interval: interval((11, 50), (12, 30))
                }
            ),
        ]
    );
    assert_eq!(day.credited(), before + hours(1.));
}

#[test]
fn a_range_inside_a_correction_keeps_both_of_its_ends() {
    let day = Day::new();
    day.save(
        ManualAdjustment::exclusion(
            id("afternoon"),
            workday(),
            interval((12, 30), (15, 0)),
            at(17, 0),
        )
        .unwrap(),
    );
    let before = day.credited();

    day.record((13, 0), (14, 0), CREDITED).unwrap();

    let kinds: Vec<_> = day
        .stored()
        .iter()
        .skip(1)
        .map(|adjustment| (adjustment.id().clone(), adjustment.kind().clone()))
        .collect();
    assert_eq!(
        kinds,
        [
            (
                id("range.1"),
                AdjustmentKind::Exclusion {
                    interval: interval((12, 30), (13, 0))
                }
            ),
            (
                id("range.2"),
                AdjustmentKind::Exclusion {
                    interval: interval((14, 0), (15, 0)),
                }
            ),
        ]
    );
    assert_eq!(day.credited(), before + hours(1.));
}

/// The kinds of the corrections the range recorded after `skip` saved ones.
fn recorded(day: &Day, skip: usize) -> Vec<(AdjustmentId, AdjustmentKind)> {
    day.stored()
        .iter()
        .skip(skip)
        .map(|adjustment| (adjustment.id().clone(), adjustment.kind().clone()))
        .collect()
}

#[test]
fn a_range_inside_an_interval_of_work_keeps_its_far_end_as_work() {
    let day = Day::new();
    day.save(
        ManualAdjustment::interval_work(
            id("lunch-call"),
            workday(),
            interval((11, 50), (12, 30)),
            at(17, 0),
        )
        .unwrap(),
    );
    let before = day.credited();

    day.record((12, 0), (12, 10), EXCLUDED).unwrap();

    assert_eq!(
        recorded(&day, 1),
        [
            (
                id("range.1"),
                AdjustmentKind::IntervalWork {
                    interval: interval((11, 50), (12, 0))
                }
            ),
            (
                id("range.2"),
                AdjustmentKind::IntervalWork {
                    interval: interval((12, 10), (12, 30))
                }
            ),
        ]
    );
    assert_eq!(day.credited(), before - SignedDuration::from_mins(10));
}

/// An interval of work recorded over activity, as one saved before the activity was known:
/// its far end could not be recorded as work there, so it counts the same as a reclassification.
#[test]
fn a_far_end_over_activity_is_kept_as_a_reclassification() {
    let day = Day::new();
    day.save(
        ManualAdjustment::interval_work(
            id("lunch-call"),
            workday(),
            interval((12, 0), (14, 0)),
            at(17, 0),
        )
        .unwrap(),
    );
    let before = day.credited();

    day.record((12, 10), (12, 20), EXCLUDED).unwrap();

    assert_eq!(
        recorded(&day, 1),
        [
            (
                id("range.1"),
                AdjustmentKind::IntervalWork {
                    interval: interval((12, 0), (12, 10))
                }
            ),
            (
                id("range.2"),
                AdjustmentKind::Reclassification {
                    interval: interval((12, 20), (14, 0)),
                    classification: CREDITED,
                    confirms_review_id: None,
                }
            ),
        ]
    );
    assert_eq!(day.credited(), before - SignedDuration::from_mins(10));
}

#[test]
fn a_range_touching_a_long_lock_settles_the_whole_lock() {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    let before = day.credited();

    let outcome = day.record((10, 30), (11, 0), EXCLUDED).unwrap();

    assert!(outcome.replaced);
    assert_eq!(day.credited(), before - hours(2.));
    assert_eq!(count(day.connection(), "LongLockResolution"), 1);
}

#[test]
fn a_retry_records_nothing_twice() {
    let day = Day::new();
    day.record((15, 0), (16, 0), CREDITED).unwrap();
    let credited = day.credited();

    day.record((15, 0), (16, 0), CREDITED).unwrap();

    assert_eq!(count(day.connection(), "ManualAdjustment"), 1);
    assert_eq!(day.credited(), credited);
}

#[test]
fn a_range_cannot_undo_a_lock_confirmed_as_work() {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    day.record((9, 0), (11, 0), CREDITED).unwrap();

    let refused = day.record((10, 0), (10, 30), EXCLUDED);

    assert!(matches!(refused, Err(RangeError::Bound(_))), "{refused:?}");
}

#[test]
fn a_range_already_counted_as_asked_records_nothing() {
    let day = Day::new();

    let outcome = day.record((9, 0), (10, 0), CREDITED).unwrap();

    assert_eq!(outcome, RangeOutcome::default());
    assert_eq!(count(day.connection(), "ManualAdjustment"), 0);
}

#[test]
fn a_refused_range_settles_no_lock() {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    day.lock((15, 30), (17, 30));
    day.record((15, 30), (17, 30), CREDITED).unwrap();

    let refused = day.record((9, 0), (17, 30), EXCLUDED);

    assert!(matches!(refused, Err(RangeError::Bound(_))), "{refused:?}");
    assert_eq!(count(day.connection(), "LongLockResolution"), 1);
}

#[test]
fn a_lock_with_a_correction_inside_settles_either_way() {
    for (classification, credited) in [(EXCLUDED, -2.), (CREDITED, 0.)] {
        let day = Day::new();
        day.lock((9, 0), (11, 0));
        let before = day.credited();
        day.save(
            ManualAdjustment::exclusion(
                id("coffee"),
                workday(),
                interval((9, 30), (10, 0)),
                at(17, 0),
            )
            .unwrap(),
        );

        day.record((9, 0), (11, 0), classification).unwrap();

        assert_eq!(
            day.credited(),
            before + hours(credited),
            "{classification:?}"
        );
        assert_eq!(count(day.connection(), "LongLockResolution"), 1);
    }
}

#[test]
fn a_batch_recorded_again_adds_nothing() {
    let day = Day::new();
    day.save(
        ManualAdjustment::exclusion(
            id("afternoon"),
            workday(),
            interval((12, 30), (15, 0)),
            at(17, 0),
        )
        .unwrap(),
    );
    let command = RangeCommand {
        workday: workday(),
        range: interval((13, 0), (14, 0)),
        classification: CREDITED,
        id: id("range"),
        created_at: day.clock.now(),
    };
    day.with(|accounting, _| {
        let calculation = accounting
            .calculate(MONDAY, &accounting.capture_context())
            .unwrap();
        let current = calculation.summary_input.accounting();
        let records = plan(current, &day.stored(), &command).unwrap().records();
        let repository = ManualAdjustmentRepository::new(day.connection());
        for _ in 0..2 {
            repository.append_all_validated(current, &records).unwrap();
        }
    });

    assert_eq!(count(day.connection(), "ManualAdjustment"), 4);
}

#[test]
fn a_lock_still_going_on_is_refused_before_anything_is_written() {
    let day = Day::new();
    SessionEventRepository::new(day.connection(), &FrozenClock::new(at(16, 0)))
        .record(
            OperationKey::new("lock-open").unwrap(),
            WorkdayTransition::new(
                at(16, 0),
                "UTC",
                WorkdayTransitionKind::SessionObserved(SessionState::Locked),
            )
            .unwrap(),
        )
        .unwrap();
    day.save(
        ManualAdjustment::exclusion(
            id("coffee"),
            workday(),
            interval((16, 30), (17, 0)),
            at(17, 0),
        )
        .unwrap(),
    );

    let refused = day.record((16, 0), (17, 0), CREDITED);

    assert!(
        matches!(refused, Err(RangeError::Review(ReviewError::StillOpen(_)))),
        "{refused:?}"
    );
    assert_eq!(count(day.connection(), "ManualAdjustment"), 1);
    assert_eq!(count(day.connection(), "LongLockResolution"), 0);
}

#[test]
fn counting_a_lock_with_sleep_inside_leaves_the_sleep_uncounted() {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    day.sleep((9, 30), (10, 0));
    let before = day.credited();

    day.record((9, 0), (11, 0), CREDITED).unwrap();
    // Asked again, the range finds nothing left to count.
    day.record((9, 0), (11, 0), CREDITED).unwrap();

    assert_eq!(day.credited(), before);
    assert_eq!(count(day.connection(), "LongLockResolution"), 1);
}

#[test]
fn a_range_refused_at_its_last_step_leaves_its_locks_unsettled() {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    // An id the range's own corrections will take, already used by another correction.
    day.save(
        ManualAdjustment::interval_work(
            id("range.1"),
            workday(),
            interval((16, 0), (16, 30)),
            at(17, 0),
        )
        .unwrap(),
    );

    let refused = day.record((9, 0), (12, 0), EXCLUDED);

    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(count(day.connection(), "LongLockResolution"), 0);
    assert_eq!(count(day.connection(), "ManualAdjustment"), 1);
}

#[test]
fn a_correction_reaching_out_of_a_lock_keeps_its_rest_once_the_lock_is_settled() {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    day.save(
        ManualAdjustment::exclusion(
            id("errand"),
            workday(),
            interval((10, 30), (11, 30)),
            at(17, 0),
        )
        .unwrap(),
    );
    let before = day.credited();

    day.record((9, 0), (11, 0), CREDITED).unwrap();

    // The lock counts whole; the half hour after it stays excluded.
    assert_eq!(day.credited(), before + hours(0.5));
    assert_eq!(count(day.connection(), "LongLockResolution"), 1);
    let rest: Vec<UtcInterval> = day
        .stored()
        .iter()
        .filter(|adjustment| adjustment.supersedes_id() == Some(&id("errand")))
        .filter_map(ManualAdjustment::interval)
        .collect();
    assert_eq!(rest, [interval((11, 0), (11, 30))]);
}

#[test]
fn a_range_past_a_lock_takes_a_correction_reaching_out_of_it_in_one_go() {
    for ((from, to), (start, end), kept) in [
        (((10, 30), (11, 30)), ((9, 0), (11, 30)), None),
        (
            ((8, 45), (9, 30)),
            ((8, 50), (11, 0)),
            Some(interval((8, 45), (8, 50))),
        ),
    ] {
        let day = Day::new();
        day.lock((9, 0), (11, 0));
        day.save(
            ManualAdjustment::exclusion(id("errand"), workday(), interval(from, to), at(17, 0))
                .unwrap(),
        );

        day.record(start, end, CREDITED).unwrap();

        assert_eq!(count(day.connection(), "LongLockResolution"), 1);
        let replaced: Vec<Option<UtcInterval>> = day
            .stored()
            .iter()
            .filter(|adjustment| adjustment.supersedes_id() == Some(&id("errand")))
            .map(ManualAdjustment::interval)
            .collect();
        assert_eq!(replaced, [kept], "{from:?}–{to:?}");
    }
}

impl Day {
    /// « J'ai travaillé » over `from`–`to`, as the day's corrections record it.
    fn credit(&self, from: (i8, i8), to: (i8, i8)) -> Result<(), CorrectionError> {
        self.with(|accounting, _| {
            ManualAdjustmentCommands::new(
                accounting,
                ManualAdjustmentRepository::new(self.connection()),
            )
            .save(
                &ManualAdjustment::interval_work(
                    id(&format!("credit-{}-{}", from.0, from.1)),
                    workday(),
                    interval(from, to),
                    at(17, 0),
                )
                .unwrap(),
            )
            .map(|_| ())
        })
    }

    /// « Oui, je travaillais » on the day's one lock to review.
    fn confirm_from_review(&self) -> Result<(), ReviewError> {
        self.with(|accounting, _| {
            let calculation = accounting
                .calculate(MONDAY, &accounting.capture_context())
                .unwrap();
            let [review] = calculation.summary_input.accounting().pending_reviews() else {
                panic!("the day has one lock to review");
            };
            LongLockReviews::new(
                ManualAdjustmentRepository::new(self.connection()),
                SessionEventRepository::new(self.connection(), &self.clock),
                LongLockResolutionRepository::new(self.connection()),
            )
            .resolve(
                accounting,
                &workday(),
                &review.id,
                ManualResolution::ConfirmWork,
            )
            .map(|_| ())
        })
    }
}

/// Lock 09:00–11:00 with sleep 09:30–10:00 inside it, on a day of 475 min of activity.
fn day_with_sleep_in_a_lock() -> Day {
    let day = Day::new();
    day.lock((9, 0), (11, 0));
    day.sleep((9, 30), (10, 0));
    assert_eq!(day.credited(), SignedDuration::from_mins(445));
    day
}

#[test]
fn counting_a_lock_keeps_the_sleep_the_user_credited_inside_it() {
    for (from, to) in [((9, 0), (11, 0)), ((9, 30), (10, 0)), ((10, 15), (10, 45))] {
        let day = day_with_sleep_in_a_lock();
        day.credit((9, 30), (10, 0)).unwrap();
        assert_eq!(day.credited(), SignedDuration::from_mins(475));

        day.record(from, to, CREDITED).unwrap();

        assert_eq!(
            day.credited(),
            SignedDuration::from_mins(475),
            "{from:?}–{to:?}"
        );
        assert_eq!(count(day.connection(), "LongLockResolution"), 1);
    }
}

#[test]
fn confirming_a_lock_from_its_review_keeps_the_sleep_the_user_credited_inside_it() {
    let day = day_with_sleep_in_a_lock();
    day.credit((9, 30), (10, 0)).unwrap();

    day.confirm_from_review().unwrap();

    assert_eq!(day.credited(), SignedDuration::from_mins(475));
    assert_eq!(count(day.connection(), "LongLockResolution"), 1);
}

#[test]
fn the_sleep_inside_a_confirmed_lock_can_still_be_credited() {
    let day = day_with_sleep_in_a_lock();
    day.confirm_from_review().unwrap();
    assert_eq!(day.credited(), SignedDuration::from_mins(445));

    day.credit((9, 30), (10, 0)).unwrap();

    assert_eq!(day.credited(), SignedDuration::from_mins(475));
}

#[test]
fn a_confirmed_lock_still_refuses_an_exclusion_inside_it() {
    let day = day_with_sleep_in_a_lock();
    day.confirm_from_review().unwrap();

    let refused = day.with(|accounting, _| {
        ManualAdjustmentCommands::new(
            accounting,
            ManualAdjustmentRepository::new(day.connection()),
        )
        .save(
            &ManualAdjustment::exclusion(
                id("coffee"),
                workday(),
                interval((10, 15), (10, 45)),
                day.clock.now(),
            )
            .unwrap(),
        )
    });

    assert!(
        matches!(
            refused,
            Err(CorrectionError::Adjustments(ManualAdjustmentError::Validation(
                worktime_domain::adjustments::AdjustmentValidationError::OverlappingActiveAdjustments
            )))
        ),
        "{refused:?}"
    );
    assert_eq!(day.credited(), SignedDuration::from_mins(445));
}
