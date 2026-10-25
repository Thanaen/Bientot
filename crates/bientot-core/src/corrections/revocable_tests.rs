//! How each adjustment of one history stands to its day's long-lock reviews, and what of it the
//! ledger revokes.

use super::*;
use crate::reviews::{ManualResolution, adjustment_id_for};

/// What the ledger revokes of one Monday's history: a correction and its edits, but never a
/// revocation, nor a long-lock resolution, whether its id or its kind names the review, nor an
/// edit of one.
#[test]
fn only_a_correction_bound_to_no_review_is_revocable() {
    let monday = utc_day(TRACKED_MONDAY);
    let lock = interval(tracked_at(9), tracked_at(10));
    let at = |minutes| tracked_at(11) + SignedDuration::from_mins(minutes);
    let meeting = duration_work("meeting", TRACKED_MONDAY, hours(0.5), at(0));
    let longer_meeting = duration_work("longer-meeting", TRACKED_MONDAY, hours(1.0), at(1))
        .superseding(meeting.id().clone());
    let confirmation = ManualAdjustment::long_lock_confirmation(
        id("confirmed-elsewhere"),
        monday.clone(),
        lock,
        at(2),
        LongLockReviewId::new("session-lock:42").unwrap(),
    )
    .unwrap();
    let confirmation_edit = ManualAdjustment::reclassification(
        id("confirmation-edit"),
        monday.clone(),
        lock,
        ManualClassification::Credited,
        at(3),
    )
    .unwrap()
    .superseding(confirmation.id().clone());
    let resolution = ManualAdjustment::reclassification(
        id("long-lock-review:2026-08-24:UTC:session-lock:43:exclude"),
        monday.clone(),
        interval(tracked_at(10), tracked_at(11)),
        ManualClassification::Excluded,
        at(4),
    )
    .unwrap();
    let lunch = duration_work("lunch", TRACKED_MONDAY, hours(0.25), at(5));
    let revocation =
        ManualAdjustment::revocation(id("revoke-lunch"), monday, lunch.id().clone(), at(6))
            .unwrap();
    let cases = [
        (meeting, Binding::Free),
        (longer_meeting, Binding::Free),
        (confirmation, Binding::ConfirmsReview),
        (confirmation_edit, Binding::FromResolution),
        (resolution, Binding::FromResolution),
        (lunch, Binding::Free),
        (revocation, Binding::Revocation),
    ];
    let history = history_by_id(cases.iter().map(|(adjustment, _)| adjustment.clone()));

    assert_eq!(
        cases
            .iter()
            .map(|(adjustment, _)| (adjustment.id().as_str(), binding(adjustment, &history)))
            .collect::<Vec<_>>(),
        cases
            .iter()
            .map(|(adjustment, bound)| (adjustment.id().as_str(), *bound))
            .collect::<Vec<_>>()
    );
}

/// A confirmation recorded from a review carries the id of a resolution as well: it still names
/// the review it resolves.
#[test]
fn a_confirmation_with_the_id_of_a_resolution_confirms_its_review() {
    let monday = utc_day(TRACKED_MONDAY);
    let review = LongLockReviewId::new("session-lock:42").unwrap();
    let confirmation = ManualAdjustment::long_lock_confirmation(
        adjustment_id_for(&monday, &review, ManualResolution::ConfirmWork),
        monday,
        interval(tracked_at(9), tracked_at(10)),
        tracked_at(11),
        review,
    )
    .unwrap();
    assert!(is_resolution_adjustment(confirmation.id()));

    let history = history_by_id([confirmation.clone()]);
    assert_eq!(binding(&confirmation, &history), Binding::ConfirmsReview);
}
