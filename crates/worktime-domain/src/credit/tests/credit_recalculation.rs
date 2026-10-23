use super::*;

/// Opening, applied, earned (also the new overtime), closing and expiring credit, in minutes.
fn balances(result: &DailyCreditResult) -> (i64, i64, i64, i64, i64) {
    assert_eq!(result.new_overtime(), result.earned_transferable_credit());
    assert_eq!(
        result.opening_balance() - result.applied_credit() + result.earned_transferable_credit(),
        result.closing_balance()
    );
    (
        result.opening_balance().as_mins(),
        result.applied_credit().as_mins(),
        result.earned_transferable_credit().as_mins(),
        result.closing_balance().as_mins(),
        result.credit_expiring_after_friday().as_mins(),
    )
}

#[test]
fn correcting_monday_recalculates_the_literal_week_without_mutating_prior_data() {
    let original_inputs = [
        day(0, 600),
        day(1, 400),
        day(2, 400),
        day(3, 400),
        day(4, 400),
    ];
    let original_input_snapshot = original_inputs;
    let original = calculate_week(&original_inputs, &settings()).unwrap();
    let original_snapshot = original.clone();

    let corrected_inputs = [
        day(0, 480),
        day(1, 400),
        day(2, 400),
        day(3, 400),
        day(4, 400),
    ];
    let corrected = calculate_week(&corrected_inputs, &settings()).unwrap();

    assert_eq!(original_inputs, original_input_snapshot);
    assert_eq!(original, original_snapshot);
    assert_eq!(
        corrected,
        calculate_week(&corrected_inputs, &settings()).unwrap()
    );

    assert_eq!(
        original.iter().map(balances).collect::<Vec<_>>(),
        [
            (0, 0, 180, 180, 0),
            (180, 60, 40, 160, 0),
            (160, 60, 40, 140, 0),
            (140, 60, 40, 120, 0),
            (120, 60, 40, 100, 100),
        ]
    );
    assert_eq!(
        corrected.iter().map(balances).collect::<Vec<_>>(),
        [
            (0, 0, 60, 60, 0),
            (60, 60, 40, 40, 0),
            (40, 40, 20, 20, 0),
            (20, 20, 0, 0, 0),
            (0, 0, 0, 0, 0),
        ]
    );
    assert_eq!(
        corrected[1..]
            .iter()
            .map(|result| result.effective_target().as_mins())
            .collect::<Vec<_>>(),
        [360, 380, 400, 420]
    );
}

/// The .NET test also checks that the result list rejects writes; a returned `Vec` is owned by
/// the caller, so only the independence from the inputs remains.
#[test]
fn returned_days_do_not_retain_the_mutable_input_collection() {
    let mut inputs = [day(0, 450), day(1, 390)];
    let results = calculate_week(&inputs, &settings()).unwrap();

    inputs[0] = day(0, 0);

    assert_eq!(inputs[0].gross_credited_work, SignedDuration::ZERO);
    assert_eq!(results[0].gross_credited_work(), minutes(450));
    assert_eq!(results[1].opening_balance(), minutes(30));
}
