//! Ported from `LongLockReviewServiceTests.cs`, the long-lock cases of
//! `TrackingControlIntegrationTests.cs` and `EarlierDayReviewOfTheCurrentWeekIsResolvedFromHistory`
//! of `HistoryIntegrationTests.cs`, against a migrated SQLite ledger instead of the .NET in-memory
//! stores. The .NET call counters of the in-memory stores are replaced by the rows the ledger
//! holds, and its failing store by a temporary trigger that refuses adjustment inserts.

use std::sync::Barrier;

use bientot_domain::accounting::{
    AccountingInput, AccountingLabel, AccountingPolicy, ActivityCoverage, ActivitySegment,
    ActivityState, AutomaticOrigin, Recorded, SegmentKind, SessionState, WorkdayEventHistory,
    WorkdayEventId, WorkdayStateAnchors, calculate,
};
use bientot_domain::adjustments::AdjustmentValidationError;
use bientot_domain::history::DailySummaryDisposition;
use bientot_domain::sources::SourceHealth;
use bientot_domain::time::FrozenClock;
use bientot_store::activity::ActivityError;
use bientot_store::credit::WeeklyCreditRepository;
use bientot_store::history::DailySummaryRepository;
use bientot_store::sessions::RecordTransitionOutcome;
use jiff::SignedDuration;
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use rusqlite::Connection;

use super::*;
use crate::history::HistoryService;
use crate::sources::{ActivityReadResult, ActivitySource};
use crate::test_support::{
    Database, FullyActive, count, fail_adjustment_inserts, restore_adjustment_inserts,
    utc_accounting, utc_zones,
};
use crate::weekly::WeeklyRecalculation;

const DAY: Date = date(2026, 8, 3);
const MONDAY: Date = date(2026, 8, 24);

fn on(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn at(hour: i8, minute: i8) -> Timestamp {
    on(DAY, hour, minute)
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(DAY, "UTC")
}

fn review_id() -> LongLockReviewId {
    LongLockReviewId::new("session-lock:42").unwrap()
}

fn locked_episode() -> UtcInterval {
    UtcInterval::new(at(8, 0), at(9, 30)).unwrap()
}

fn adjustment_id(value: &str) -> AdjustmentId {
    AdjustmentId::new(value).unwrap()
}

fn adjustment_command(id: &str) -> LongLockReviewAdjustmentCommand {
    LongLockReviewAdjustmentCommand {
        adjustment_id: adjustment_id(id),
        review_id: review_id(),
        locked_episode: locked_episode(),
        created_at: at(9, 31),
    }
}

fn finish_command() -> FinishLongLockReviewCommand {
    FinishLongLockReviewCommand {
        review_id: review_id(),
        locked_episode: locked_episode(),
    }
}

fn origin(value: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(value).unwrap()
}

/// The .NET `Result` helper: 2026-08-03 accounted from 08:00, active throughout, with the lock
/// of event 42 starting at 08:00 and lasting until `through` when `close_lock` is set.
struct Day {
    adjustments: Vec<ManualAdjustment>,
    review_threshold: SignedDuration,
    finished_at_lock_start: bool,
    through: Timestamp,
    close_lock: bool,
}

impl Default for Day {
    fn default() -> Self {
        Self {
            adjustments: Vec::new(),
            review_threshold: SignedDuration::from_mins(90),
            finished_at_lock_start: false,
            through: at(9, 30),
            close_lock: true,
        }
    }
}

impl Day {
    fn calculate(self) -> AccountingResult {
        let window = WorkdayWindow {
            workday: workday(),
            start: at(8, 0),
            end: at(17, 0),
        };
        let bounds = UtcInterval::new(window.start, window.end).unwrap();
        let activity = ActivityCoverage::new(
            bounds,
            [ActivitySegment::new(
                bounds,
                ActivityState::Active,
                origin("activitywatch"),
            )],
        );
        let mut transitions: Vec<RecordedTransition> = vec![
            Recorded::new(
                WorkdayEventId(42),
                at(8, 0),
                SessionState::Locked,
                origin("windows:session"),
            )
            .into(),
        ];
        if self.close_lock {
            transitions.push(
                Recorded::new(
                    WorkdayEventId(44),
                    self.through,
                    SessionState::Unlocked,
                    origin("windows:session"),
                )
                .into(),
            );
        }
        if self.finished_at_lock_start {
            transitions.push(
                Recorded::new(
                    WorkdayEventId(43),
                    at(8, 0),
                    TrackingState::Finished,
                    origin("user"),
                )
                .into(),
            );
        }
        let input = AccountingInput::new(
            window,
            self.through,
            AccountingPolicy::new(
                UtcInterval::new(at(12, 0), at(14, 0)).unwrap(),
                self.review_threshold,
            )
            .unwrap(),
            activity,
            WorkdayEventHistory::new(WorkdayStateAnchors::default(), transitions),
            self.adjustments,
        )
        .unwrap();
        calculate(&input).unwrap()
    }
}

fn pending_day() -> AccountingResult {
    Day::default().calculate()
}

fn reviews<'a>(connection: &'a Connection, clock: &'a FrozenClock) -> LongLockReviews<'a> {
    LongLockReviews::new(
        ManualAdjustmentRepository::new(connection),
        SessionEventRepository::new(connection, clock),
        LongLockResolutionRepository::new(connection),
    )
}

fn stored_adjustments(connection: &Connection, workday: &LocalWorkday) -> Vec<ManualAdjustment> {
    ManualAdjustmentRepository::new(connection)
        .read_for_workday(workday)
        .unwrap()
}

#[derive(Debug, PartialEq, Eq)]
struct StoredClaim {
    review_id: String,
    kind: String,
    identity: String,
    created_at: String,
}

fn claims(connection: &Connection) -> Vec<StoredClaim> {
    connection
        .prepare(
            "SELECT ReviewId, ActionKind, ActionIdentity, CanonicalCreatedAtUtc \
             FROM LongLockResolution ORDER BY ReviewId",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(StoredClaim {
                review_id: row.get(0)?,
                kind: row.get(1)?,
                identity: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Operation key, instant, zone and kind of every ledger row, in insertion order.
fn event_rows(connection: &Connection) -> Vec<[String; 4]> {
    connection
        .prepare(
            "SELECT OperationKey, OccurredAtUtc, TimezoneId, Kind FROM SessionEvent ORDER BY Id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?])
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn assert_nothing_written(connection: &Connection) {
    assert_eq!(
        [
            count(connection, "ManualAdjustment"),
            count(connection, "SessionEvent"),
            count(connection, "LongLockResolution"),
        ],
        [0, 0, 0]
    );
}

fn inserted(id: &str) -> AppendAdjustmentResult {
    AppendAdjustmentResult {
        outcome: AppendAdjustmentOutcome::Inserted,
        authoritative_id: adjustment_id(id),
    }
}

fn already_recorded(id: &AdjustmentId) -> AppendAdjustmentResult {
    AppendAdjustmentResult {
        outcome: AppendAdjustmentOutcome::AlreadyRecorded,
        authoritative_id: id.clone(),
    }
}

#[test]
fn confirm_as_work_persists_credited_reclassification_for_the_complete_episode() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));

    let result = reviews(&database.connection, &clock)
        .confirm_as_work(&pending_day(), &adjustment_command("confirm-42"))
        .unwrap();

    assert_eq!(result, inserted("confirm-42"));
    assert_eq!(
        stored_adjustments(&database.connection, &workday()),
        [ManualAdjustment::long_lock_confirmation(
            adjustment_id("confirm-42"),
            workday(),
            locked_episode(),
            at(9, 31),
            review_id(),
        )
        .unwrap()]
    );
    assert_eq!(
        claims(&database.connection),
        [StoredClaim {
            review_id: "session-lock:42".into(),
            kind: "confirm-work".into(),
            identity: "confirm-42\u{1F}2026-08-03T08:00:00.0000000+00:00\u{1F}\
                       2026-08-03T09:30:00.0000000+00:00\u{1F}Credited"
                .into(),
            created_at: "2026-08-03T09:31:00.0000000+00:00".into(),
        }]
    );
}

#[test]
fn reclassify_as_not_work_persists_excluded_reclassification() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));

    reviews(&database.connection, &clock)
        .reclassify_as_not_work(&pending_day(), &adjustment_command("exclude-42"))
        .unwrap();

    let stored = stored_adjustments(&database.connection, &workday());
    assert_eq!(
        stored,
        [ManualAdjustment::reclassification(
            adjustment_id("exclude-42"),
            workday(),
            locked_episode(),
            ManualClassification::Excluded,
            at(9, 31),
        )
        .unwrap()]
    );
    let replayed = Day {
        adjustments: stored,
        ..Day::default()
    }
    .calculate();
    assert_eq!(replayed.pending_reviews(), []);
}

#[test]
fn finish_at_lock_start_persists_only_finished_tracking_with_deterministic_key() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));

    let result = reviews(&database.connection, &clock)
        .finish_at_lock_start(&pending_day(), &finish_command())
        .unwrap();

    assert_eq!(result.outcome, RecordTransitionOutcome::Inserted);
    assert_eq!(count(&database.connection, "ManualAdjustment"), 0);
    assert_eq!(
        event_rows(&database.connection),
        [[
            "long-lock-review:session-lock:42:finish-at-lock-start",
            "2026-08-03T08:00:00.0000000+00:00",
            "UTC",
            "day-finished",
        ]
        .map(String::from)]
    );
    assert_eq!(
        claims(&database.connection),
        [StoredClaim {
            review_id: "session-lock:42".into(),
            kind: "finish-at-lock-start".into(),
            identity: "long-lock-review:session-lock:42:finish-at-lock-start\u{1F}\
                       2026-08-03T08:00:00.0000000+00:00\u{1F}\
                       2026-08-03T09:30:00.0000000+00:00\u{1F}UTC\u{1F}Finished"
                .into(),
            created_at: "2026-08-03T08:00:00.0000000+00:00".into(),
        }]
    );
    let finished = Day {
        finished_at_lock_start: true,
        ..Day::default()
    }
    .calculate();
    assert_eq!(finished.pending_reviews(), []);
}

#[test]
fn missing_review_fails_with_typed_error_without_writing() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let not_pending = Day {
        review_threshold: SignedDuration::from_hours(4),
        ..Day::default()
    }
    .calculate();

    let error = reviews(&database.connection, &clock)
        .confirm_as_work(&not_pending, &adjustment_command("missing"))
        .unwrap_err();

    assert!(matches!(error, ReviewError::NotPending(id) if id == review_id()));
    assert_nothing_written(&database.connection);
}

#[test]
fn open_review_rejects_every_resolution_before_claim() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let service = reviews(&database.connection, &clock);
    let current = Day {
        close_lock: false,
        ..Day::default()
    }
    .calculate();
    assert!(matches!(current.pending_reviews(), [review] if review.is_open));

    let errors = [
        service
            .confirm_as_work(&current, &adjustment_command("open-confirm"))
            .map(|_| ()),
        service
            .reclassify_as_not_work(&current, &adjustment_command("open-exclude"))
            .map(|_| ()),
        service
            .finish_at_lock_start(&current, &finish_command())
            .map(|_| ()),
    ];

    for error in errors {
        assert!(matches!(error, Err(ReviewError::StillOpen(id)) if id == review_id()));
    }
    assert_nothing_written(&database.connection);
}

#[test]
fn changed_locked_episode_fails_with_typed_error_without_writing() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let command = LongLockReviewAdjustmentCommand {
        locked_episode: UtcInterval::new(at(8, 0), at(9, 0)).unwrap(),
        ..adjustment_command("stale")
    };

    let error = reviews(&database.connection, &clock)
        .reclassify_as_not_work(&pending_day(), &command)
        .unwrap_err();

    assert!(matches!(error, ReviewError::LockedEpisodeChanged(id) if id == review_id()));
    assert_nothing_written(&database.connection);
}

#[test]
fn manual_resolution_validates_against_stored_history_before_append() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let conflicting = ManualAdjustment::exclusion(
        adjustment_id("existing"),
        workday(),
        UtcInterval::new(at(8, 30), at(9, 0)).unwrap(),
        at(9, 1),
    )
    .unwrap();
    ManualAdjustmentRepository::new(&database.connection)
        .append(&conflicting)
        .unwrap();

    let error = reviews(&database.connection, &clock)
        .confirm_as_work(&pending_day(), &adjustment_command("blocked"))
        .unwrap_err();

    assert!(matches!(
        error,
        ReviewError::Adjustments(ManualAdjustmentError::Validation(
            AdjustmentValidationError::OverlappingActiveAdjustments
        ))
    ));
    assert_eq!(
        stored_adjustments(&database.connection, &workday()),
        [conflicting]
    );
    assert_eq!(claims(&database.connection), []);
}

#[test]
fn exact_manual_retry_is_idempotent_even_after_review_replay_clears_pending_state() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let service = reviews(&database.connection, &clock);
    let command = adjustment_command("confirm-retry");

    let first = service.confirm_as_work(&pending_day(), &command).unwrap();
    let replayed = Day {
        adjustments: stored_adjustments(&database.connection, &workday()),
        ..Day::default()
    }
    .calculate();
    let retry = service.confirm_as_work(&replayed, &command).unwrap();

    assert_eq!(first, inserted("confirm-retry"));
    assert_eq!(retry, already_recorded(&command.adjustment_id));
    assert_eq!(count(&database.connection, "ManualAdjustment"), 1);
    assert_eq!(replayed.pending_reviews(), []);
}

#[test]
fn transient_append_failure_keeps_claim_and_same_action_retry_resumes() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let service = reviews(&database.connection, &clock);
    let command = adjustment_command("confirm-after-transient");

    fail_adjustment_inserts(&database.connection);
    let failure = service
        .confirm_as_work(&pending_day(), &command)
        .unwrap_err();
    let conflict = service
        .finish_at_lock_start(&pending_day(), &finish_command())
        .unwrap_err();
    restore_adjustment_inserts(&database.connection);
    let retry = service.confirm_as_work(&pending_day(), &command).unwrap();

    assert!(matches!(
        failure,
        ReviewError::Adjustments(ManualAdjustmentError::Storage(_))
    ));
    let ReviewError::Resolutions(LongLockResolutionError::Conflict(conflict)) = conflict else {
        panic!("expected a resolution conflict, got {conflict:?}");
    };
    assert_eq!(
        conflict.existing_action.kind(),
        LongLockResolutionActionKind::ConfirmWork
    );
    assert_eq!(retry, inserted("confirm-after-transient"));
    assert_eq!(
        [
            count(&database.connection, "ManualAdjustment"),
            count(&database.connection, "LongLockResolution"),
            count(&database.connection, "SessionEvent"),
        ],
        [1, 1, 0]
    );
}

#[test]
fn retry_rejects_a_review_whose_locked_episode_extended_after_the_first_command() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let service = reviews(&database.connection, &clock);
    let command = adjustment_command("confirm-before-extension");
    service.confirm_as_work(&pending_day(), &command).unwrap();
    let extended = Day {
        adjustments: stored_adjustments(&database.connection, &workday()),
        through: at(10, 0),
        ..Day::default()
    }
    .calculate();

    let error = service.confirm_as_work(&extended, &command).unwrap_err();

    assert!(matches!(error, ReviewError::LockedEpisodeChanged(id) if id == review_id()));
    assert!(matches!(
        extended.pending_reviews(),
        [review] if review.locked_episode == UtcInterval::new(at(8, 0), at(10, 0)).unwrap()
    ));
    assert_eq!(count(&database.connection, "ManualAdjustment"), 1);
}

/// The retry runs later than the first finish; its transition still names the lock start.
/// Not in the .NET tests: two callers of one action share its claim, and the one whose append
/// fails releases it while the other records the resolution. Steps A and B run the store calls
/// `append_resolution` makes, in both orders.
#[test]
fn a_failed_caller_cannot_release_a_claim_another_caller_of_the_action_records_under() {
    for release_first in [true, false] {
        let database = Database::new();
        let resolutions = LongLockResolutionRepository::new(&database.connection);
        let adjustments = ManualAdjustmentRepository::new(&database.connection);
        let claim_of = |kind, identity: &str| LongLockResolutionClaim {
            workday: workday(),
            review_id: review_id(),
            action: LongLockResolutionAction::new(kind, identity).unwrap(),
            proposed_created_at: at(9, 31),
        };
        let confirm = claim_of(LongLockResolutionActionKind::ConfirmWork, "confirm");
        let finish = claim_of(LongLockResolutionActionKind::FinishAtLockStart, "finish");
        let resolution = ManualAdjustment::long_lock_confirmation(
            adjustment_id("confirm-42"),
            workday(),
            locked_episode(),
            at(9, 31),
            review_id(),
        )
        .unwrap();
        resolutions.claim(&confirm).unwrap();
        resolutions.claim(&confirm).unwrap();

        if release_first {
            assert!(
                adjustments
                    .release_unrecorded(&confirm, &resolution)
                    .unwrap()
            );
            assert!(matches!(
                adjustments.append_claimed(&pending_day(), &resolution, &confirm),
                Err(ManualAdjustmentError::ClaimReleased(id)) if id == *resolution.id()
            ));
            resolutions.claim(&finish).unwrap();
            assert!(stored_adjustments(&database.connection, &workday()).is_empty());
            assert_eq!(claims(&database.connection)[0].kind, "finish-at-lock-start");
        } else {
            adjustments
                .append_claimed(&pending_day(), &resolution, &confirm)
                .unwrap();
            assert!(
                !adjustments
                    .release_unrecorded(&confirm, &resolution)
                    .unwrap()
            );
            assert!(matches!(
                resolutions.claim(&finish),
                Err(LongLockResolutionError::Conflict(_))
            ));
            assert_eq!(
                stored_adjustments(&database.connection, &workday()),
                [resolution]
            );
            assert_eq!(claims(&database.connection)[0].kind, "confirm-work");
        }
        assert_eq!(claims(&database.connection).len(), 1);
    }
}

/// Not in the .NET tests: a claim released and acquired again by the same action is a new
/// generation, which a delayed cleanup of the earlier one leaves alone.
#[test]
fn a_delayed_cleanup_leaves_the_claim_the_action_acquired_again() {
    let database = Database::new();
    let resolutions = LongLockResolutionRepository::new(&database.connection);
    let adjustments = ManualAdjustmentRepository::new(&database.connection);
    let claim_at = |kind, identity: &str, proposed_created_at| LongLockResolutionClaim {
        workday: workday(),
        review_id: review_id(),
        action: LongLockResolutionAction::new(kind, identity).unwrap(),
        proposed_created_at,
    };
    let resolution_at = |created_at| {
        ManualAdjustment::long_lock_confirmation(
            adjustment_id("confirm-42"),
            workday(),
            locked_episode(),
            created_at,
            review_id(),
        )
        .unwrap()
    };
    let first = claim_at(
        LongLockResolutionActionKind::ConfirmWork,
        "confirm",
        at(9, 31),
    );
    let again = claim_at(
        LongLockResolutionActionKind::ConfirmWork,
        "confirm",
        at(9, 45),
    );

    resolutions.claim(&first).unwrap();
    assert!(
        adjustments
            .release_unrecorded(&first, &resolution_at(at(9, 31)))
            .unwrap()
    );
    let reacquired = resolutions.claim(&again).unwrap();
    assert_eq!(reacquired.canonical_created_at, at(9, 45));
    adjustments
        .append_claimed(&pending_day(), &resolution_at(at(9, 45)), &again)
        .unwrap();
    let delayed = adjustments
        .release_unrecorded(&first, &resolution_at(at(9, 31)))
        .unwrap();

    assert!(!delayed);
    assert_eq!(
        claims(&database.connection)[0].created_at,
        "2026-08-03T09:45:00.0000000+00:00"
    );
    assert!(matches!(
        resolutions.claim(&claim_at(
            LongLockResolutionActionKind::FinishAtLockStart,
            "finish",
            at(9, 50),
        )),
        Err(LongLockResolutionError::Conflict(_))
    ));
}

/// Not in the .NET tests: a caller of an earlier generation cannot record under a claim the
/// action released and acquired again.
#[test]
fn a_resolution_of_an_earlier_claim_generation_is_not_recorded() {
    let database = Database::new();
    let resolutions = LongLockResolutionRepository::new(&database.connection);
    let adjustments = ManualAdjustmentRepository::new(&database.connection);
    let claim_at = |proposed_created_at| LongLockResolutionClaim {
        workday: workday(),
        review_id: review_id(),
        action: LongLockResolutionAction::new(LongLockResolutionActionKind::ConfirmWork, "confirm")
            .unwrap(),
        proposed_created_at,
    };
    let stale = ManualAdjustment::long_lock_confirmation(
        adjustment_id("confirm-42"),
        workday(),
        locked_episode(),
        at(9, 31),
        review_id(),
    )
    .unwrap();
    resolutions.claim(&claim_at(at(9, 31))).unwrap();
    resolutions.release(&claim_at(at(9, 31))).unwrap();
    resolutions.claim(&claim_at(at(9, 45))).unwrap();

    assert!(matches!(
        adjustments.append_claimed(&pending_day(), &stale, &claim_at(at(9, 31))),
        Err(ManualAdjustmentError::ClaimReleased(_))
    ));
    assert!(stored_adjustments(&database.connection, &workday()).is_empty());
}

/// Not in the .NET tests: an id already taken by an unrelated adjustment fails the resolution
/// without keeping the review claimed.
#[test]
fn an_unrelated_adjustment_under_the_resolution_id_does_not_keep_the_review_claimed() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let tuesday = LocalWorkday::new(date(2026, 8, 4), "UTC");
    let unrelated = ManualAdjustment::duration_work(
        adjustment_id("used-id"),
        tuesday,
        SignedDuration::from_hours(1),
        on(date(2026, 8, 4), 12, 0),
    )
    .unwrap();
    ManualAdjustmentRepository::new(&database.connection)
        .append(&unrelated)
        .unwrap();

    let taken = reviews(&database.connection, &clock)
        .confirm_as_work(&pending_day(), &adjustment_command("used-id"));
    assert!(matches!(
        taken,
        Err(ReviewError::Adjustments(ManualAdjustmentError::IdConflict(id))) if id == adjustment_id("used-id")
    ));
    assert!(claims(&database.connection).is_empty());

    let connection = database.reopen();
    let resolved = reviews(&connection, &clock)
        .confirm_as_work(&pending_day(), &adjustment_command("confirm-42"))
        .unwrap();

    assert_eq!(resolved, inserted("confirm-42"));
    assert_eq!(claims(&connection).len(), 1);
    assert_eq!(stored_adjustments(&connection, &workday()).len(), 1);
}

#[test]
fn exact_finish_retry_uses_the_same_operation_key() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let service = reviews(&database.connection, &clock);

    let first = service
        .finish_at_lock_start(&pending_day(), &finish_command())
        .unwrap();
    let replayed = Day {
        finished_at_lock_start: true,
        ..Day::default()
    }
    .calculate();
    clock.set(at(12, 0));
    let retry = service
        .finish_at_lock_start(&replayed, &finish_command())
        .unwrap();

    assert_eq!(first.outcome, RecordTransitionOutcome::Inserted);
    assert_eq!(
        retry,
        RecordTransitionResult {
            outcome: RecordTransitionOutcome::AlreadyRecorded,
            authoritative_id: first.authoritative_id,
        }
    );
    assert_eq!(count(&database.connection, "SessionEvent"), 1);
    assert_eq!(replayed.pending_reviews(), []);
}

#[test]
fn resolution_adjustment_ids_are_deterministic_per_day_zone_review_and_action() {
    let review = LongLockReviewId::new("session-lock:42").unwrap();
    let ids = [
        (LocalWorkday::new(MONDAY, "UTC"), ManualResolution::Exclude),
        (
            LocalWorkday::new(MONDAY, "Romance Standard Time"),
            ManualResolution::ConfirmWork,
        ),
        (
            LocalWorkday::new(MONDAY, "???>>>"),
            ManualResolution::ConfirmWork,
        ),
        (LocalWorkday::new(MONDAY, "a"), ManualResolution::Exclude),
    ]
    .map(|(workday, resolution)| adjustment_id_for(&workday, &review, resolution));

    assert_eq!(
        ids.each_ref().map(AdjustmentId::as_str),
        [
            "long-lock-review:2026-08-24:VVRD:session-lock:42:exclude",
            "long-lock-review:2026-08-24:Um9tYW5jZSBTdGFuZGFyZCBUaW1l:session-lock:42:confirm-work",
            "long-lock-review:2026-08-24:Pz8_Pj4-:session-lock:42:confirm-work",
            "long-lock-review:2026-08-24:YQ:session-lock:42:exclude",
        ]
    );
    assert!(ids.iter().all(is_resolution_adjustment));
    assert!(!is_resolution_adjustment(&adjustment_id(
        "long-lock-review"
    )));
}

fn monday_at(hour: i8) -> Timestamp {
    on(MONDAY, hour, 0)
}

fn record_session(connection: &Connection, key: &str, at: Timestamp, state: SessionState) {
    let clock = FrozenClock::new(at);
    SessionEventRepository::new(connection, &clock)
        .record(
            OperationKey::new(key).unwrap(),
            WorkdayTransition::new(at, "UTC", WorkdayTransitionKind::SessionObserved(state))
                .unwrap(),
        )
        .unwrap();
}

/// A lock from Monday 00:00 to 02:00, so that its review is pending and closed from 02:00.
fn record_completed_lock(connection: &Connection) {
    record_session(connection, "lock-start", monday_at(0), SessionState::Locked);
    record_session(connection, "lock-end", monday_at(2), SessionState::Unlocked);
}

fn resolve_with(
    service: &LongLockReviews,
    current: &AccountingResult,
    command: &LongLockReviewAdjustmentCommand,
    resolution: ManualResolution,
) -> Result<AppendAdjustmentResult, ReviewError> {
    match resolution {
        ManualResolution::ConfirmWork => service.confirm_as_work(current, command),
        ManualResolution::Exclude => service.reclassify_as_not_work(current, command),
    }
}

#[test]
fn confirm_and_finish_race_has_one_durable_winner_across_restart() {
    let zones = utc_zones();
    for iteration in 0..20 {
        let database = Database::new();
        let clock = FrozenClock::new(monday_at(3));
        record_completed_lock(&database.connection);
        let pending = utc_accounting(&database.connection, &FullyActive, &clock, &zones)
            .calculate_current()
            .unwrap();
        let [review] = pending.accounting().pending_reviews() else {
            panic!("expected one pending review");
        };
        let confirmation = LongLockReviewAdjustmentCommand {
            adjustment_id: adjustment_id(&format!("race-confirm-{iteration}")),
            review_id: review.id.clone(),
            locked_episode: review.locked_episode,
            created_at: pending.computed_at(),
        };
        let finish = FinishLongLockReviewCommand {
            review_id: review.id.clone(),
            locked_episode: review.locked_episode,
        };
        let (confirm_connection, finish_connection) = (database.reopen(), database.reopen());
        let start = &Barrier::new(2);
        let current = pending.accounting();

        let (confirmed, finished) = std::thread::scope(|scope| {
            let confirm = scope.spawn(|| {
                let connection = confirm_connection;
                let clock = FrozenClock::new(monday_at(3));
                start.wait();
                reviews(&connection, &clock)
                    .confirm_as_work(current, &confirmation)
                    .map(|_| ())
            });
            let finish = scope.spawn(|| {
                let connection = finish_connection;
                let clock = FrozenClock::new(monday_at(3));
                start.wait();
                reviews(&connection, &clock)
                    .finish_at_lock_start(current, &finish)
                    .map(|_| ())
            });
            (confirm.join().unwrap(), finish.join().unwrap())
        });

        let confirm_won = match (&confirmed, &finished) {
            (Ok(()), Err(ReviewError::Resolutions(LongLockResolutionError::Conflict(_)))) => true,
            (Err(ReviewError::Resolutions(LongLockResolutionError::Conflict(_))), Ok(())) => false,
            outcomes => panic!("expected one winner and one conflict, got {outcomes:?}"),
        };
        let reopened = database.reopen();
        assert_eq!(count(&reopened, "LongLockResolution"), 1);
        let monday = LocalWorkday::new(MONDAY, "UTC");
        let adjustments = stored_adjustments(&reopened, &monday);
        let day_finished = event_rows(&reopened)
            .iter()
            .any(|[_, _, _, kind]| kind == "day-finished");
        assert_eq!(
            (adjustments.len(), day_finished),
            (usize::from(confirm_won), !confirm_won)
        );

        let replay = utc_accounting(&reopened, &FullyActive, &clock, &zones)
            .calculate_current()
            .unwrap();
        let restarted = reviews(&reopened, &clock);
        assert_eq!(replay.accounting().pending_reviews(), []);
        if confirm_won {
            assert_eq!(replay.credited_work(), SignedDuration::from_hours(3));
            assert_eq!(
                restarted
                    .confirm_as_work(replay.accounting(), &confirmation)
                    .unwrap(),
                already_recorded(&confirmation.adjustment_id)
            );
        } else {
            assert_eq!(replay.credited_work(), SignedDuration::ZERO);
            assert_eq!(
                restarted
                    .finish_at_lock_start(replay.accounting(), &finish)
                    .unwrap()
                    .outcome,
                RecordTransitionOutcome::AlreadyRecorded
            );
        }
    }
}

/// The retries run at 04:00 and 05:00, later than the first command at 03:00.
#[test]
fn manual_resolution_retry_uses_claimed_created_at_across_newer_summaries() {
    let zones = utc_zones();
    let monday = LocalWorkday::new(MONDAY, "UTC");
    for (resolution, id, classification) in [
        (
            ManualResolution::ConfirmWork,
            "restart-confirm",
            ManualClassification::Credited,
        ),
        (
            ManualResolution::Exclude,
            "restart-exclude",
            ManualClassification::Excluded,
        ),
    ] {
        let database = Database::new();
        let clock = FrozenClock::new(monday_at(3));
        record_completed_lock(&database.connection);
        let first_summary = utc_accounting(&database.connection, &FullyActive, &clock, &zones)
            .calculate_current()
            .unwrap();
        let [review] = first_summary.accounting().pending_reviews() else {
            panic!("expected one pending review");
        };
        let first_command = LongLockReviewAdjustmentCommand {
            adjustment_id: adjustment_id(id),
            review_id: review.id.clone(),
            locked_episode: review.locked_episode,
            created_at: first_summary.computed_at(),
        };

        fail_adjustment_inserts(&database.connection);
        let failure = resolve_with(
            &reviews(&database.connection, &clock),
            first_summary.accounting(),
            &first_command,
            resolution,
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            ReviewError::Adjustments(ManualAdjustmentError::Storage(_))
        ));
        assert_eq!(count(&database.connection, "LongLockResolution"), 1);
        assert_eq!(count(&database.connection, "ManualAdjustment"), 0);

        clock.set(monday_at(4));
        let second = database.reopen();
        let second_summary = utc_accounting(&second, &FullyActive, &clock, &zones)
            .calculate_current()
            .unwrap();
        assert!(matches!(
            second_summary.accounting().pending_reviews(),
            [pending] if pending.locked_episode == review.locked_episode
        ));
        let second_command = LongLockReviewAdjustmentCommand {
            created_at: second_summary.computed_at(),
            ..first_command.clone()
        };
        let inserted = resolve_with(
            &reviews(&second, &clock),
            second_summary.accounting(),
            &second_command,
            resolution,
        )
        .unwrap();

        clock.set(monday_at(5));
        let third = database.reopen();
        let replay = utc_accounting(&third, &FullyActive, &clock, &zones)
            .calculate_current()
            .unwrap();
        assert_eq!(replay.accounting().pending_reviews(), []);
        let third_command = LongLockReviewAdjustmentCommand {
            created_at: replay.computed_at(),
            ..first_command.clone()
        };
        let retry = resolve_with(
            &reviews(&third, &clock),
            replay.accounting(),
            &third_command,
            resolution,
        )
        .unwrap();

        assert_eq!(inserted.outcome, AppendAdjustmentOutcome::Inserted);
        assert_eq!(retry, already_recorded(&first_command.adjustment_id));
        let expected = match classification {
            ManualClassification::Credited => ManualAdjustment::long_lock_confirmation(
                first_command.adjustment_id.clone(),
                monday.clone(),
                review.locked_episode,
                monday_at(3),
                review.id.clone(),
            ),
            ManualClassification::Excluded => ManualAdjustment::reclassification(
                first_command.adjustment_id.clone(),
                monday.clone(),
                review.locked_episode,
                classification,
                monday_at(3),
            ),
        }
        .unwrap();
        assert_eq!(stored_adjustments(&third, &monday), [expected]);
        assert_eq!(count(&third, "LongLockResolution"), 1);
    }
}

/// The service side of `LockCarriedIntoTheNextWorkdayIsExcludedWithoutReviewActions`: the tray
/// offers no action because every action is refused, while the lock is open on Monday and once
/// Tuesday no longer has a review.
#[test]
fn lock_carried_into_the_next_workday_is_excluded_without_review_actions() {
    let database = Database::new();
    let tuesday = date(2026, 8, 25);
    let clock = FrozenClock::new(monday_at(2));
    let zones = utc_zones();
    let accounting = utc_accounting(&database.connection, &FullyActive, &clock, &zones);
    let service = reviews(&database.connection, &clock);
    record_session(
        &database.connection,
        "continuous-lock",
        monday_at(0),
        SessionState::Locked,
    );

    let monday_summary = accounting.calculate_current().unwrap();
    let [monday_review] = monday_summary.accounting().pending_reviews() else {
        panic!("expected one pending review");
    };
    assert!(monday_review.is_open);
    let command = LongLockReviewAdjustmentCommand {
        adjustment_id: adjustment_id("carried-confirm"),
        review_id: monday_review.id.clone(),
        locked_episode: monday_review.locked_episode,
        created_at: monday_at(2),
    };
    let finish = FinishLongLockReviewCommand {
        review_id: monday_review.id.clone(),
        locked_episode: monday_review.locked_episode,
    };
    assert!(matches!(
        service.confirm_as_work(monday_summary.accounting(), &command),
        Err(ReviewError::StillOpen(_))
    ));
    assert!(matches!(
        service.finish_at_lock_start(monday_summary.accounting(), &finish),
        Err(ReviewError::StillOpen(_))
    ));

    clock.set(on(tuesday, 2, 0));
    let tuesday_summary = accounting.calculate_current().unwrap();
    assert_eq!(tuesday_summary.accounting().pending_reviews(), []);
    assert_eq!(tuesday_summary.credited_work(), SignedDuration::ZERO);
    assert!(matches!(
        service.confirm_as_work(tuesday_summary.accounting(), &command),
        Err(ReviewError::EpisodeOutsideHorizon(_))
    ));
    assert!(matches!(
        service.finish_at_lock_start(tuesday_summary.accounting(), &finish),
        Err(ReviewError::NotPending(_))
    ));

    record_session(
        &database.connection,
        "continuous-lock-ended",
        on(tuesday, 2, 0),
        SessionState::Unlocked,
    );
    clock.set(on(tuesday, 3, 0));
    let unlocked = accounting.calculate_current().unwrap();
    assert_eq!(unlocked.accounting().pending_reviews(), []);
    let overnight: Vec<_> = unlocked
        .accounting()
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.label() == AccountingLabel::OvernightLock)
        .map(|segment| (segment.interval(), segment.kind().clone()))
        .collect();
    assert_eq!(
        overnight,
        [(
            UtcInterval::new(on(tuesday, 0, 0), on(tuesday, 2, 0)).unwrap(),
            SegmentKind::Excluded
        )]
    );
    assert_eq!(unlocked.credited_work(), SignedDuration::from_hours(1));
    assert_eq!(count(&database.connection, "ManualAdjustment"), 0);
    assert_eq!(count(&database.connection, "LongLockResolution"), 0);
}

/// Each weekday of the week of 2026-08-31 is active from 09:00 to 17:00 and away around it, as
/// a healthy source reports it.
struct OfficeHours;

impl ActivitySource for OfficeHours {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        let day = interval.start().to_zoned(TimeZone::UTC).date();
        let origin = || origin("activitywatch:test");
        let office = UtcInterval::new(on(day, 9, 0), on(day, 17, 0)).unwrap();
        let segments = [
            UtcInterval::new(interval.start(), office.start())
                .map(|away| ActivitySegment::new(away, ActivityState::Afk, origin())),
            interval
                .intersect(office)
                .ok_or(bientot_domain::intervals::EmptyInterval)
                .map(|active| ActivitySegment::new(active, ActivityState::Active, origin())),
            UtcInterval::new(office.end(), interval.end())
                .map(|away| ActivitySegment::new(away, ActivityState::Afk, origin())),
        ]
        .into_iter()
        .flatten();
        Ok(ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(interval, segments),
            health: SourceHealth::healthy("activitywatch:test", interval.end()),
        })
    }
}

/// .NET seeds the week with the `--demo-history-week` clock and source; the clock here is the
/// same Friday 18:00, the source a plain office day.
#[test]
fn earlier_day_review_of_the_current_week_is_resolved_from_history() {
    let zones = utc_zones();
    let friday = date(2026, 9, 4);
    let wednesday_date = date(2026, 9, 2);
    let wednesday = LocalWorkday::new(wednesday_date, "UTC");
    for (resolution, other, resolved) in [
        (
            ManualResolution::ConfirmWork,
            ManualResolution::Exclude,
            DailySummaryDisposition::ConfirmedCredit,
        ),
        (
            ManualResolution::Exclude,
            ManualResolution::ConfirmWork,
            DailySummaryDisposition::Excluded,
        ),
    ] {
        let database = Database::new();
        let connection = &database.connection;
        let clock = FrozenClock::new(on(friday, 18, 0));
        record_session(
            connection,
            "long-lock",
            on(wednesday_date, 10, 0),
            SessionState::Locked,
        );
        record_session(
            connection,
            "long-unlock",
            on(wednesday_date, 12, 0),
            SessionState::Unlocked,
        );
        record_session(
            connection,
            "short-lock",
            on(wednesday_date, 15, 0),
            SessionState::Locked,
        );
        record_session(
            connection,
            "short-unlock",
            on(wednesday_date, 15, 20),
            SessionState::Unlocked,
        );
        let accounting = utc_accounting(connection, &OfficeHours, &clock, &zones);
        let store = WeeklyCreditRepository::new(connection);
        let weekly = WeeklyRecalculation::new(&accounting, &store);
        let history = HistoryService::new(DailySummaryRepository::new(connection), &clock, &zones);
        let service = reviews(connection, &clock);
        let wednesday_snapshot = || {
            history.current_week().unwrap().days()[2]
                .latest_revision
                .clone()
                .unwrap()
                .snapshot()
                .clone()
        };

        weekly.calculate_current().unwrap();
        let before = wednesday_snapshot();
        let [pending] = before
            .breakdown()
            .entries()
            .iter()
            .filter(|entry| entry.review_id().is_some())
            .collect::<Vec<_>>()[..]
        else {
            panic!("expected one entry under review");
        };
        assert_eq!(
            (pending.disposition(), pending.duration()),
            (
                DailySummaryDisposition::ProvisionalCredit,
                SignedDuration::from_hours(2)
            )
        );
        let [short_lock] = before
            .breakdown()
            .entries()
            .iter()
            .filter(|entry| {
                entry.disposition() == DailySummaryDisposition::ProvisionalCredit
                    && entry.review_id().is_none()
            })
            .collect::<Vec<_>>()[..]
        else {
            panic!("expected one short lock");
        };
        assert_eq!(short_lock.duration(), SignedDuration::from_mins(20));
        assert!(before.has_pending_review());
        let review = pending.review_id().unwrap().clone();

        let first = service
            .resolve(&accounting, &wednesday, &review, resolution)
            .unwrap();
        weekly.calculate_current().unwrap();

        let after = wednesday_snapshot();
        assert!(!after.has_pending_review());
        assert!(
            after
                .breakdown()
                .entries()
                .iter()
                .all(|entry| entry.review_id().is_none())
        );
        assert!(after.breakdown().entries().contains(short_lock));
        assert!(
            after
                .breakdown()
                .entries()
                .iter()
                .any(|entry| entry.disposition() == resolved)
        );
        let expected_id = adjustment_id_for(&wednesday, &review, resolution);
        assert_eq!(first.authoritative_id, expected_id);
        assert_eq!(first.outcome, AppendAdjustmentOutcome::Inserted);

        clock.set(on(friday, 19, 0));
        assert_eq!(
            service
                .resolve(&accounting, &wednesday, &review, resolution)
                .unwrap(),
            already_recorded(&expected_id)
        );
        assert!(matches!(
            service.resolve(&accounting, &wednesday, &review, other),
            Err(ReviewError::NotPending(id)) if id == review
        ));
        for outside in [date(2026, 8, 28), date(2026, 9, 5)] {
            assert!(matches!(
                service.resolve(
                    &accounting,
                    &LocalWorkday::new(outside, "UTC"),
                    &review,
                    resolution
                ),
                Err(ReviewError::OutsideCurrentWeek)
            ));
        }
        assert_eq!(count(connection, "ManualAdjustment"), 1);
        assert_eq!(count(connection, "LongLockResolution"), 1);
    }
}

#[test]
fn an_appointment_review_cannot_finish_the_day_and_nothing_is_written() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let day = pending_day();
    let reviews_of_appointment = day
        .pending_reviews()
        .iter()
        .cloned()
        .map(|review| LongLockReview {
            reason: LongLockReviewReason::RecurringAppointment,
            ..review
        })
        .collect();
    let day = day.with_pending_reviews(reviews_of_appointment);

    let error = reviews(&database.connection, &clock)
        .finish_at_lock_start(&day, &finish_command())
        .unwrap_err();

    assert!(matches!(error, ReviewError::NotALock(id) if id == review_id()));
    assert_nothing_written(&database.connection);
}

#[test]
fn a_rebuilt_session_review_cannot_finish_the_day_and_nothing_is_written() {
    let database = Database::new();
    let clock = FrozenClock::new(at(9, 31));
    let day = pending_day();
    let rebuilt_reviews = day
        .pending_reviews()
        .iter()
        .cloned()
        .map(|review| LongLockReview {
            reason: LongLockReviewReason::RebuiltSession,
            ..review
        })
        .collect();
    let day = day.with_pending_reviews(rebuilt_reviews);

    let error = reviews(&database.connection, &clock)
        .finish_at_lock_start(&day, &finish_command())
        .unwrap_err();

    assert!(matches!(error, ReviewError::NotALock(id) if id == review_id()));
    assert_nothing_written(&database.connection);
}

#[test]
fn resolution_of_reads_back_the_review_and_the_answer_of_its_adjustment() {
    let workday = LocalWorkday::new(DAY, "Europe/Paris");
    let review = LongLockReviewId::for_appointment("thursday:meeting", on(DAY, 14, 0));
    for resolution in [ManualResolution::ConfirmWork, ManualResolution::Exclude] {
        let id = adjustment_id_for(&workday, &review, resolution);
        assert_eq!(resolution_of(&id), Some((review.clone(), resolution)));
    }
    let other = AdjustmentId::new("manual:2026-08-03:declare").unwrap();
    assert_eq!(resolution_of(&other), None);
}
