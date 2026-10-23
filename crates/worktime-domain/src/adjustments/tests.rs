use jiff::civil::date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::accounting::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AutomaticOrigin, PowerState, Recorded, SegmentKind, SessionState, WorkdayEventHistory,
    WorkdayEventId, WorkdayStateAnchors, calculate,
};
use crate::time::WorkdayWindow;

fn at(hour: i8, minute: i8) -> Timestamp {
    date(2026, 8, 3)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn between(start_hour: i8, end_hour: i8) -> UtcInterval {
    UtcInterval::new(at(start_hour, 0), at(end_hour, 0)).unwrap()
}

fn workday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 8, 3), "UTC")
}

fn id(value: &str) -> AdjustmentId {
    AdjustmentId::new(value).unwrap()
}

fn origin(id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(id).unwrap()
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn hours(hours: i64) -> SignedDuration {
    SignedDuration::from_hours(hours)
}

fn activity(interval: UtcInterval, state: ActivityState) -> ActivitySegment {
    ActivitySegment::new(interval, state, origin("activitywatch:afk"))
}

fn suspended_history() -> WorkdayEventHistory {
    WorkdayEventHistory::new(
        WorkdayStateAnchors {
            power: Some(Recorded::new(
                WorkdayEventId(1),
                at(7, 0),
                PowerState::Suspended,
                origin("windows:power"),
            )),
            ..WorkdayStateAnchors::default()
        },
        [],
    )
}

fn calculate_with(
    activity: &[ActivitySegment],
    adjustments: &[&ManualAdjustment],
    history: WorkdayEventHistory,
) -> AccountingResult {
    let bounds = between(8, 11);
    let window = WorkdayWindow {
        workday: workday(),
        start: bounds.start(),
        end: bounds.end(),
    };
    let input = AccountingInput::new(
        window,
        bounds.end(),
        AccountingPolicy::new(between(12, 14), minutes(90)).unwrap(),
        ActivityCoverage::new(bounds, activity.iter().cloned()),
        history,
        adjustments
            .iter()
            .map(|&adjustment| adjustment.clone())
            .collect(),
    )
    .unwrap();
    calculate(&input).unwrap()
}

fn calculate_day(
    activity: &[ActivitySegment],
    adjustments: &[&ManualAdjustment],
) -> AccountingResult {
    calculate_with(activity, adjustments, WorkdayEventHistory::default())
}

fn interval_work(value: &str, interval: UtcInterval, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::interval_work(id(value), workday(), interval, created_at).unwrap()
}

fn duration_work(value: &str, duration: SignedDuration, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::duration_work(id(value), workday(), duration, created_at).unwrap()
}

fn exclusion(value: &str, interval: UtcInterval, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::exclusion(id(value), workday(), interval, created_at).unwrap()
}

fn credit(value: &str, interval: UtcInterval, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::reclassification(
        id(value),
        workday(),
        interval,
        ManualClassification::Credited,
        created_at,
    )
    .unwrap()
}

fn confirmation(value: &str, interval: UtcInterval, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::long_lock_confirmation(
        id(value),
        workday(),
        interval,
        created_at,
        LongLockReviewId::new("session-lock:1").unwrap(),
    )
    .unwrap()
}

fn revocation(value: &str, revokes: &ManualAdjustment, created_at: Timestamp) -> ManualAdjustment {
    ManualAdjustment::revocation(id(value), workday(), revokes.id().clone(), created_at).unwrap()
}

fn history(adjustments: &[&ManualAdjustment]) -> Vec<ManualAdjustment> {
    adjustments
        .iter()
        .map(|&adjustment| adjustment.clone())
        .collect()
}

fn manual_segments(result: &AccountingResult) -> Vec<(UtcInterval, &str)> {
    result
        .timeline()
        .segments()
        .iter()
        .filter_map(|segment| {
            let id = segment.origin().manual_adjustment_id()?;
            Some((segment.interval(), id.as_str()))
        })
        .collect()
}

#[test]
fn interval_work_overlapping_credited_coverage_is_rejected_without_implicit_split() {
    let adjustment = interval_work("manual-1", between(9, 11), at(12, 0));

    let current = calculate_day(&[activity(between(8, 10), ActivityState::Active)], &[]);
    let error = validate_for_append(&current, &[], &adjustment).unwrap_err();

    assert_eq!(
        error,
        AdjustmentValidationError::OverlapsCreditedCoverage { id: id("manual-1") }
    );
}

#[test]
fn interval_work_overlapping_provisional_credit_is_also_rejected() {
    let session = |event, occurred_at, state| {
        Recorded::new(
            WorkdayEventId(event),
            occurred_at,
            state,
            origin("windows:session"),
        )
        .into()
    };
    let locked = WorkdayEventHistory::new(
        WorkdayStateAnchors::default(),
        [
            session(1, at(8, 0), SessionState::Locked),
            session(2, at(10, 0), SessionState::Unlocked),
        ],
    );
    let adjustment = interval_work("manual-1", between(8, 9), at(12, 0));

    let current = calculate_with(&[], &[], locked);
    let error = validate_for_append(&current, &[], &adjustment).unwrap_err();

    assert_eq!(
        error,
        AdjustmentValidationError::OverlapsCreditedCoverage { id: id("manual-1") }
    );
}

#[test]
fn interval_work_credits_only_previously_uncredited_coverage() {
    let adjustment = interval_work("manual-1", between(10, 11), at(12, 0));
    let evidence = [activity(between(8, 10), ActivityState::Active)];
    let current = calculate_day(&evidence, &[]);

    validate_for_append(&current, &[], &adjustment).unwrap();
    let result = calculate_day(&evidence, &[&adjustment]);

    assert_eq!(result.credited_work(), hours(3));
    let manual: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| segment.interval() == between(10, 11))
        .collect();
    let [manual] = manual.as_slice() else {
        panic!("expected one segment over the adjustment");
    };
    assert_eq!(manual.kind(), &SegmentKind::Credited);
    assert_eq!(manual.label(), AccountingLabel::Manual);
    assert_eq!(
        manual.origin().manual_adjustment_id(),
        Some(&id("manual-1"))
    );
}

#[test]
fn duration_work_remains_outside_timeline_and_counts_exactly_once() {
    let adjustment = duration_work("duration-1", minutes(30), at(12, 0));

    let result = calculate_day(&[], &[&adjustment, &adjustment]);

    let [credit] = result.manual_duration_credits() else {
        panic!("expected one duration credit");
    };
    assert_eq!(credit.adjustment_id(), &id("duration-1"));
    assert_eq!(credit.duration(), minutes(30));
    assert_eq!(result.credited_work(), minutes(30));
    assert!(manual_segments(&result).is_empty());
}

#[test]
fn conflicting_reuse_of_an_adjustment_id_is_rejected() {
    let original = duration_work("duration-1", minutes(30), at(12, 0));
    let conflict = duration_work("duration-1", minutes(45), at(12, 0));

    let current = calculate_day(&[], &[&original]);
    let error = validate_for_append(&current, &history(&[&original]), &conflict).unwrap_err();

    assert_eq!(
        error,
        AdjustmentValidationError::ConflictingId {
            id: id("duration-1")
        }
    );
}

#[test]
fn supersession_leaves_only_the_latest_adjustment_active() {
    let original = duration_work("duration-1", minutes(30), at(12, 0));
    let correction =
        duration_work("duration-2", minutes(45), at(12, 1)).superseding(original.id().clone());

    let current = calculate_day(&[], &[&original]);
    validate_for_append(&current, &history(&[&original]), &correction).unwrap();
    let result = calculate_day(&[], &[&correction, &original]);

    let [credit] = result.manual_duration_credits() else {
        panic!("expected one duration credit");
    };
    assert_eq!(credit.adjustment_id(), correction.id());
    assert_eq!(result.credited_work(), minutes(45));
}

#[test]
fn a_history_cannot_have_two_active_successors_for_one_adjustment() {
    let original = duration_work("duration-1", minutes(30), at(12, 0));
    let first_correction =
        duration_work("duration-2", minutes(45), at(12, 1)).superseding(original.id().clone());
    let second_correction =
        duration_work("duration-3", minutes(60), at(12, 2)).superseding(original.id().clone());

    let current = calculate_day(&[], &[&original, &first_correction]);
    let error = validate_for_append(
        &current,
        &history(&[&original, &first_correction]),
        &second_correction,
    )
    .unwrap_err();

    assert_eq!(error, AdjustmentValidationError::MultipleActiveSuccessors);
}

#[test]
fn command_validation_rejects_overlapping_active_interval_adjustments() {
    let existing = exclusion("exclude-1", between(8, 10), at(12, 0));
    let candidate = credit("reclassify-1", between(9, 11), at(12, 1));
    let current = calculate_day(&[], &[&existing]);

    let error = validate_for_append(&current, &history(&[&existing]), &candidate).unwrap_err();

    assert_eq!(
        error,
        AdjustmentValidationError::OverlappingActiveAdjustments
    );
}

#[test]
fn manual_exclusion_and_reclassification_override_automatic_classification_deterministically() {
    let exclusion = exclusion("exclude-1", between(8, 9), at(12, 0));
    let reclassification = credit("reclassify-1", between(9, 10), at(12, 0));
    let evidence = [activity(between(8, 9), ActivityState::Active)];

    let first = calculate_day(&evidence, &[&exclusion, &reclassification]);
    let second = calculate_day(&evidence, &[&reclassification, &exclusion]);

    assert_eq!(first.credited_work(), second.credited_work());
    assert_eq!(first.timeline().segments(), second.timeline().segments());
    let segments = first.timeline().segments();
    assert_eq!(segments[0].kind(), &SegmentKind::Excluded);
    assert_eq!(segments[1].kind(), &SegmentKind::Credited);
    assert!(
        segments[..2]
            .iter()
            .all(|segment| segment.origin().manual_adjustment_id().is_some())
    );
}

#[test]
fn interval_work_credits_the_selected_part_of_suspended_coverage() {
    let adjustment =
        interval_work("manual-1", between(9, 10), at(12, 0)).with_reason("worked away from the PC");

    let current = calculate_with(&[], &[], suspended_history());
    validate_for_append(&current, &[], &adjustment).unwrap();
    let result = calculate_with(&[], &[&adjustment], suspended_history());

    assert_eq!(result.credited_work(), hours(1));
    let labels: Vec<_> = result
        .timeline()
        .segments()
        .iter()
        .map(|segment| segment.label())
        .collect();
    assert_eq!(
        labels,
        [
            AccountingLabel::Suspended,
            AccountingLabel::Manual,
            AccountingLabel::Suspended
        ]
    );
    assert_eq!(manual_segments(&result), [(between(9, 10), "manual-1")]);
}

#[test]
fn reclassification_credits_full_suspended_coverage() {
    let adjustment =
        credit("reclassify-1", between(8, 11), at(12, 0)).with_reason("off-device workshop");

    let current = calculate_with(&[], &[], suspended_history());
    validate_for_append(&current, &[], &adjustment).unwrap();
    let result = calculate_with(&[], &[&adjustment], suspended_history());

    assert_eq!(result.credited_work(), hours(3));
    let [credited] = result.timeline().segments() else {
        panic!("expected one segment");
    };
    assert_eq!(credited.kind(), &SegmentKind::Credited);
    assert_eq!(credited.label(), AccountingLabel::Manual);
    assert_eq!(
        credited.origin().manual_adjustment_id(),
        Some(adjustment.id())
    );
}

#[test]
fn persisted_interval_work_replays_after_source_becomes_credited_without_double_counting() {
    let adjustment = interval_work("manual-1", between(9, 10), at(12, 0));

    let result = calculate_day(
        &[activity(between(8, 11), ActivityState::Active)],
        &[&adjustment],
    );

    assert_eq!(result.credited_work(), hours(3));
    assert_eq!(
        result.timeline().credited_coverage().covered_duration(),
        hours(3)
    );
    assert_eq!(manual_segments(&result), [(between(9, 10), "manual-1")]);

    validate_for_append(&result, &history(&[&adjustment]), &adjustment).unwrap();
}

#[test]
fn revoking_interval_work_restores_the_automatic_timeline() {
    let evidence = [activity(between(8, 10), ActivityState::Active)];
    let work = interval_work("manual-1", between(10, 11), at(12, 0));
    let revocation = revocation("revoke-1", &work, at(12, 1));

    validate_for_append(
        &calculate_day(&evidence, &[&work]),
        &history(&[&work]),
        &revocation,
    )
    .unwrap();
    let result = calculate_day(&evidence, &[&work, &revocation]);

    assert_eq!(result.credited_work(), hours(2));
    assert_eq!(
        result.timeline().segments(),
        calculate_day(&evidence, &[]).timeline().segments()
    );
    assert!(manual_segments(&result).is_empty());
}

#[test]
fn revoking_an_exclusion_credits_the_activity_again() {
    let evidence = [activity(between(8, 11), ActivityState::Active)];
    let exclusion = exclusion("exclude-1", between(8, 9), at(12, 0));
    let revocation = revocation("revoke-1", &exclusion, at(12, 1));

    assert_eq!(
        calculate_day(&evidence, &[&exclusion]).credited_work(),
        hours(2)
    );
    let result = calculate_day(&evidence, &[&exclusion, &revocation]);

    assert_eq!(result.credited_work(), hours(3));
    assert_eq!(
        result.timeline().segments(),
        calculate_day(&evidence, &[]).timeline().segments()
    );
}

#[test]
fn revoking_duration_work_removes_its_credit() {
    let duration = duration_work("duration-1", minutes(30), at(12, 0));
    let revocation = revocation("revoke-1", &duration, at(12, 1));

    let result = calculate_day(&[], &[&duration, &revocation]);

    assert!(result.manual_duration_credits().is_empty());
    assert_eq!(result.credited_work(), SignedDuration::ZERO);
}

#[test]
fn a_revocation_cannot_be_revoked() {
    let work = interval_work("manual-1", between(10, 11), at(12, 0));
    let first = revocation("revoke-1", &work, at(12, 1));
    let second = revocation("revoke-2", &first, at(12, 2));

    let error = validate_for_append(
        &calculate_day(&[], &[&work, &first]),
        &history(&[&work, &first]),
        &second,
    )
    .unwrap_err();

    assert_eq!(
        error,
        AdjustmentValidationError::InvalidRevocationTarget {
            revocation_id: id("revoke-2"),
            target_id: id("revoke-1"),
        }
    );
}

#[test]
fn a_long_lock_confirmation_cannot_be_revoked() {
    let confirmation = confirmation("confirm-1", between(9, 10), at(12, 0));
    let revocation = revocation("revoke-1", &confirmation, at(12, 1));

    let error = validate_for_append(
        &calculate_day(&[], &[&confirmation]),
        &history(&[&confirmation]),
        &revocation,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        AdjustmentValidationError::InvalidRevocationTarget { .. }
    ));
}

#[test]
fn an_edited_long_lock_confirmation_cannot_be_revoked() {
    let confirmation = confirmation("confirm-1", between(9, 10), at(12, 0));
    let edit = credit("edit-1", between(9, 10), at(12, 1))
        .superseding(confirmation.id().clone())
        .with_reason("meeting");
    let revocation = revocation("revoke-1", &edit, at(12, 2));

    let error = validate_for_append(
        &calculate_day(&[], &[&confirmation, &edit]),
        &history(&[&confirmation, &edit]),
        &revocation,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        AdjustmentValidationError::InvalidRevocationTarget { .. }
    ));
}

#[test]
fn an_adjustment_can_be_revoked_only_once() {
    let work = interval_work("manual-1", between(10, 11), at(12, 0));
    let first = revocation("revoke-1", &work, at(12, 1));
    let second = revocation("revoke-2", &work, at(12, 2));

    let error = validate_for_append(
        &calculate_day(&[], &[&work, &first]),
        &history(&[&work, &first]),
        &second,
    )
    .unwrap_err();

    assert_eq!(error, AdjustmentValidationError::MultipleActiveSuccessors);
}

#[test]
fn an_adjustment_superseding_a_revocation_becomes_the_active_correction() {
    let work = interval_work("manual-1", between(10, 11), at(12, 0));
    let revocation = revocation("revoke-1", &work, at(12, 1));
    let replacement =
        interval_work("manual-2", between(9, 10), at(12, 2)).superseding(revocation.id().clone());

    validate_for_append(
        &calculate_day(&[], &[&work, &revocation]),
        &history(&[&work, &revocation]),
        &replacement,
    )
    .unwrap();
    let result = calculate_day(&[], &[&work, &revocation, &replacement]);

    assert_eq!(result.credited_work(), hours(1));
    assert_eq!(manual_segments(&result), [(between(9, 10), "manual-2")]);
}

#[test]
fn constructors_reject_what_the_dotnet_guards_reject() {
    let saturday = LocalWorkday::new(date(2026, 8, 1), "UTC");

    assert_eq!(
        ManualAdjustment::exclusion(id("a"), saturday, between(8, 9), at(12, 0)),
        Err(InvalidAdjustment::NotAWorkingDay)
    );
    assert_eq!(
        ManualAdjustment::duration_work(id("a"), workday(), SignedDuration::ZERO, at(12, 0)),
        Err(InvalidAdjustment::NonPositiveDuration)
    );
    assert_eq!(
        ManualAdjustment::new(
            id("a"),
            workday(),
            at(12, 0),
            None,
            AdjustmentKind::Reclassification {
                interval: between(8, 9),
                classification: ManualClassification::Excluded,
                confirms_review_id: Some(LongLockReviewId::new("session-lock:1").unwrap()),
            },
        ),
        Err(InvalidAdjustment::ConfirmationMustCredit)
    );
    assert_eq!(
        ManualAdjustment::new(
            id("a"),
            workday(),
            at(12, 0),
            None,
            AdjustmentKind::Revocation
        ),
        Err(InvalidAdjustment::RevocationWithoutTarget)
    );
    assert_eq!(id("  padded  ").as_str(), "padded");
    assert_eq!(
        credit("a", between(8, 9), at(12, 0))
            .with_reason("   ")
            .reason(),
        None
    );
}
