use std::collections::BTreeMap;

use super::*;
use CreditLedgerEntryKind::{Applied, Earned, Expired};

fn observed_at() -> Timestamp {
    utc(day_of_week(2), 15)
}

fn calculate(monday: Date, gross_minutes: &[i64]) -> Vec<DailyCreditResult> {
    let inputs: Vec<_> = (0..)
        .zip(gross_minutes)
        .map(|(offset, &gross)| DailyCreditInput::new(days_after(monday, offset), minutes(gross)))
        .collect();
    calculate_week(&inputs, &settings()).unwrap()
}

fn build(
    results: &[DailyCreditResult],
    revisions: &BTreeMap<Date, i64>,
    observed_at: Timestamp,
    previous: &[CreditLedgerEntry],
) -> Vec<CreditLedgerEntry> {
    build_ledger(
        results,
        revisions,
        &friday_window(results[0].date()),
        observed_at,
        previous,
    )
    .unwrap()
}

fn shape(entry: &CreditLedgerEntry) -> (Date, CreditLedgerEntryKind, i64, Date, i64) {
    let (date, kind, amount, origin, revision) = movement(entry);
    (date, kind, amount.as_mins(), origin, revision)
}

#[test]
fn monday_overtime_is_applied_to_tuesday_from_its_durable_origin() {
    let results = calculate(MONDAY, &[450, 390]);

    let ledger = build(
        &results,
        &revisions_of(&results, &[11, 12]),
        observed_at(),
        &[],
    );

    assert_eq!(
        ledger.iter().map(shape).collect::<Vec<_>>(),
        [
            (MONDAY, Earned, 30, MONDAY, 11),
            (day_of_week(1), Applied, 30, MONDAY, 11),
        ]
    );
}

#[test]
fn fifo_consumption_splits_across_oldest_lots_before_adding_todays_earnings() {
    let results = calculate(MONDAY, &[520, 450, 390]);

    let ledger = build(
        &results,
        &revisions_of(&results, &[1, 2, 3]),
        observed_at(),
        &[],
    );
    let wednesday = |kind| {
        ledger
            .iter()
            .filter(|entry| entry.workday_date() == day_of_week(2) && entry.kind() == kind)
            .map(shape)
            .collect::<Vec<_>>()
    };

    assert_eq!(
        wednesday(Applied),
        [
            (day_of_week(2), Applied, 40, MONDAY, 1),
            (day_of_week(2), Applied, 20, day_of_week(1), 2),
        ]
    );
    assert_eq!(
        wednesday(Earned),
        [(day_of_week(2), Earned, 30, day_of_week(2), 3)]
    );
}

#[test]
fn corrected_monday_changes_downstream_allocations_and_movement_identities() {
    let original_results = calculate(MONDAY, &[520, 450, 390]);
    let original = build(
        &original_results,
        &revisions_of(&original_results, &[1, 1, 1]),
        observed_at(),
        &[],
    );
    let corrected_results = calculate(MONDAY, &[480, 450, 390]);

    let corrected = build(
        &corrected_results,
        &revisions_of(&corrected_results, &[2, 2, 2]),
        observed_at(),
        &original,
    );

    assert_eq!(
        corrected
            .iter()
            .filter(|entry| entry.workday_date() == day_of_week(2) && entry.kind() == Applied)
            .map(shape)
            .collect::<Vec<_>>(),
        [(day_of_week(2), Applied, 60, day_of_week(1), 2)]
    );
    assert!(
        corrected
            .iter()
            .all(|entry| original.iter().all(|previous| previous.id() != entry.id()))
    );
    assert!(
        corrected
            .iter()
            .all(|entry| entry.created_at() == observed_at())
    );
}

#[test]
fn identical_rebuild_preserves_deterministic_ids_and_creation_instants() {
    let results = calculate(MONDAY, &[520, 450, 390]);
    let revisions = revisions_of(&results, &[4, 5, 6]);
    let first_observed_at = observed_at() - SignedDuration::from_hours(1);
    let first = build(&results, &revisions, first_observed_at, &[]);

    let rebuilt = build(&results, &revisions, observed_at(), &first);

    assert_eq!(first, rebuilt);
    assert!(
        rebuilt
            .iter()
            .all(|entry| entry.created_at() == first_observed_at)
    );
    assert_eq!(
        first[0].id().as_str(),
        "weekly-credit:v1:20260907:20260907:r4:earned:t60000000000:20260907:r4"
    );
}

#[test]
fn one_tick_earned_and_applied_conserves_exactly() {
    let settings = settings();
    let results = calculate_week(
        &[
            DailyCreditInput::new(MONDAY, settings.nominal_daily_target + TICK),
            DailyCreditInput::new(day_of_week(1), settings.nominal_daily_target - TICK),
        ],
        &settings,
    )
    .unwrap();

    let ledger = build(
        &results,
        &revisions_of(&results, &[1, 1]),
        observed_at(),
        &[],
    );

    assert_eq!(
        ledger
            .iter()
            .map(CreditLedgerEntry::amount)
            .collect::<Vec<_>>(),
        [TICK, TICK]
    );
    assert_eq!(sum(&ledger, Earned), sum(&ledger, Applied));
    assert!(ledger[0].id().as_str().contains(":t1:"));
}

#[test]
fn friday_credit_expires_only_at_or_after_the_explicit_window_end() {
    let results = calculate(MONDAY, &[600, 400, 400, 400, 400]);
    let revisions = revisions_of(&results, &[1, 2, 3, 4, 5]);
    let window = friday_window(MONDAY);
    let before = build_ledger(&results, &revisions, &window, window.end - TICK, &[]).unwrap();

    let at_end = build_ledger(&results, &revisions, &window, window.end, &before).unwrap();

    assert!(before.iter().all(|entry| entry.kind() != Expired));
    assert_eq!(sum(&at_end, Expired), minutes(100));
    assert_eq!(
        sum(&at_end, Earned),
        sum(&at_end, Applied) + sum(&at_end, Expired)
    );
    for entry in &at_end {
        if entry.kind() == Expired {
            assert_eq!(entry.created_at(), window.end);
            assert!(at_end.iter().any(|earned| {
                earned.kind() == Earned
                    && earned.workday_date() == entry.origin().workday_date
                    && earned.origin().revision == entry.origin().revision
            }));
        } else {
            let previous = before
                .iter()
                .find(|previous| previous.id() == entry.id())
                .unwrap();
            assert_eq!(entry.created_at(), previous.created_at());
        }
    }
}

#[test]
fn a_new_monday_starts_with_no_carry_from_the_prior_week() {
    let next_monday = days_after(MONDAY, 7);
    let results = calculate(next_monday, &[420]);

    let ledger = build(&results, &revisions_of(&results, &[1]), observed_at(), &[]);

    assert_eq!(ledger, []);
    assert_eq!(results[0].opening_balance(), SignedDuration::ZERO);
}

#[test]
fn rejects_mixed_weeks_missing_or_invalid_revisions_and_wrong_friday_window() {
    let monday = calculate(MONDAY, &[450])[0];
    let next_tuesday = calculate(days_after(MONDAY, 7), &[420, 420])[1];
    let valid_results = calculate(MONDAY, &[450, 390]);
    let valid_revisions = revisions_of(&valid_results, &[1, 2]);
    let reject = |results: &[DailyCreditResult], revisions: &BTreeMap<Date, i64>, window| {
        build_ledger(results, revisions, &window, observed_at(), &[]).unwrap_err()
    };

    assert_eq!(
        reject(
            &[monday, next_tuesday],
            &valid_revisions,
            friday_window(MONDAY)
        ),
        InvalidLedgerInput::NotConsecutive
    );
    assert_eq!(
        reject(
            &valid_results,
            &BTreeMap::from([(MONDAY, 1)]),
            friday_window(MONDAY)
        ),
        InvalidLedgerInput::MissingRevision
    );
    assert_eq!(
        reject(
            &valid_results,
            &BTreeMap::from([(MONDAY, 1), (day_of_week(1), 0)]),
            friday_window(MONDAY)
        ),
        InvalidLedgerInput::MissingRevision
    );
    assert_eq!(
        reject(
            &valid_results,
            &valid_revisions,
            friday_window(days_after(MONDAY, 7))
        ),
        InvalidLedgerInput::InvalidFridayWindow
    );
}

#[test]
fn rejects_results_that_do_not_conserve_the_lots() {
    let monday_only = calculate(MONDAY, &[450]);
    let tuesday_alone = calculate(MONDAY, &[480, 390])[1];
    let revisions = BTreeMap::from([(MONDAY, 1), (day_of_week(1), 1)]);

    assert_eq!(
        build_ledger(
            &[monday_only[0], tuesday_alone],
            &revisions,
            &friday_window(MONDAY),
            observed_at(),
            &[],
        ),
        Err(InvalidLedgerInput::OpeningBalanceMismatch)
    );
}
