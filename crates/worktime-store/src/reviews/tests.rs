use jiff::civil::{Date, date};

use super::*;
use crate::test_support::TemporaryDatabase;

fn monday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 8, 24), "UTC")
}

fn at(day: Date, hour: i8) -> Timestamp {
    day.at(hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn claim(
    workday: LocalWorkday,
    kind: LongLockResolutionActionKind,
    identity: &str,
    proposed_created_at: Option<Timestamp>,
) -> LongLockResolutionClaim {
    LongLockResolutionClaim {
        workday,
        review_id: LongLockReviewId::new("session-lock:42").unwrap(),
        action: LongLockResolutionAction::new(kind, identity).unwrap(),
        proposed_created_at: proposed_created_at.unwrap_or(at(date(2026, 8, 24), 12)),
    }
}

fn conflict(
    result: Result<ClaimLongLockResolutionResult, LongLockResolutionError>,
) -> LongLockResolutionConflict {
    match result {
        Err(LongLockResolutionError::Conflict(conflict)) => *conflict,
        other => panic!("{other:?}"),
    }
}

#[test]
fn exact_action_retry_is_idempotent_but_different_action_conflicts() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    let first_created_at = at(date(2026, 8, 24), 12);
    let confirm = claim(
        monday(),
        LongLockResolutionActionKind::ConfirmWork,
        "confirm-42",
        Some(first_created_at),
    );

    let first = repository.claim(&confirm).unwrap();
    let retry = repository
        .claim(&claim(
            monday(),
            LongLockResolutionActionKind::ConfirmWork,
            "confirm-42",
            Some(at(date(2026, 8, 24), 13)),
        ))
        .unwrap();
    let conflict = conflict(repository.claim(&claim(
        monday(),
        LongLockResolutionActionKind::FinishAtLockStart,
        "finish-42",
        None,
    )));

    assert_eq!(first.outcome, ClaimLongLockResolutionOutcome::Acquired);
    assert_eq!(
        retry.outcome,
        ClaimLongLockResolutionOutcome::AlreadyClaimed
    );
    assert_eq!(first.canonical_created_at, first_created_at);
    assert_eq!(retry.canonical_created_at, first_created_at);
    assert_eq!(conflict.existing_action, confirm.action);
    assert_eq!(
        conflict.attempted_action.kind(),
        LongLockResolutionActionKind::FinishAtLockStart
    );
    assert_eq!(database.count("LongLockResolution"), 1);
}

#[test]
fn same_kind_with_different_command_identity_conflicts() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    let original = claim(
        monday(),
        LongLockResolutionActionKind::ConfirmWork,
        "confirm-42-v1",
        None,
    );
    repository.claim(&original).unwrap();

    let conflict = conflict(repository.claim(&claim(
        monday(),
        LongLockResolutionActionKind::ConfirmWork,
        "confirm-42-v2",
        None,
    )));

    assert_eq!(conflict.existing_action, original.action);
    assert_eq!(conflict.attempted_action.identity(), "confirm-42-v2");
    assert_eq!(database.count("LongLockResolution"), 1);
}

#[test]
fn same_review_on_different_workdays_is_independent() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    let tuesday = LocalWorkday::new(date(2026, 8, 25), "UTC");

    let monday = repository
        .claim(&claim(
            monday(),
            LongLockResolutionActionKind::ConfirmWork,
            "monday-confirm",
            None,
        ))
        .unwrap();
    let next_day = repository
        .claim(&claim(
            tuesday,
            LongLockResolutionActionKind::FinishAtLockStart,
            "tuesday-finish",
            None,
        ))
        .unwrap();

    assert_eq!(monday.outcome, ClaimLongLockResolutionOutcome::Acquired);
    assert_eq!(next_day.outcome, ClaimLongLockResolutionOutcome::Acquired);
    assert_eq!(database.count("LongLockResolution"), 2);
}

#[test]
fn release_only_deletes_the_exact_claim() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    let confirm = claim(
        monday(),
        LongLockResolutionActionKind::ConfirmWork,
        "confirm-42",
        None,
    );
    repository.claim(&confirm).unwrap();

    let wrong_identity = repository
        .release(&claim(
            monday(),
            LongLockResolutionActionKind::ConfirmWork,
            "another-command",
            None,
        ))
        .unwrap();
    let released = repository.release(&confirm).unwrap();

    assert!(!wrong_identity);
    assert!(released);
    assert_eq!(database.count("LongLockResolution"), 0);
}

#[test]
fn an_action_identity_is_trimmed_and_cannot_be_blank() {
    let action =
        LongLockResolutionAction::new(LongLockResolutionActionKind::Exclude, " exclude-42 ")
            .unwrap();

    assert_eq!(action.identity(), "exclude-42");
    assert_eq!(
        LongLockResolutionAction::new(LongLockResolutionActionKind::Exclude, " \t"),
        Err(BlankIdentifier)
    );
}

#[test]
fn a_conflict_names_the_review_the_workday_and_the_existing_action() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    repository
        .claim(&claim(
            monday(),
            LongLockResolutionActionKind::Exclude,
            "exclude-42",
            None,
        ))
        .unwrap();

    let error = repository
        .claim(&claim(
            monday(),
            LongLockResolutionActionKind::ConfirmWork,
            "confirm-42",
            None,
        ))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Long-lock review 'session-lock:42' for 2026-08-24 in 'UTC' was already resolved by \
         'Exclude'."
    );
}

#[test]
fn a_sub_tick_claim_returns_the_stored_instant_on_acquisition_and_retry() {
    let database = TemporaryDatabase::new();
    let repository = LongLockResolutionRepository::new(&database.connection);
    let whole_tick = at(date(2026, 8, 24), 12);
    let confirm = claim(
        monday(),
        LongLockResolutionActionKind::ConfirmWork,
        "confirm-42",
        Some(whole_tick + jiff::SignedDuration::from_nanos(1)),
    );

    let first = repository.claim(&confirm).unwrap();
    let retry = repository.claim(&confirm).unwrap();

    assert_eq!(first.canonical_created_at, whole_tick);
    assert_eq!(retry.canonical_created_at, whole_tick);
}
