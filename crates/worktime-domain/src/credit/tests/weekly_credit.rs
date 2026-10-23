use super::*;

/// Date offset, gross, opening, applied, effective target, remaining, new overtime, earned,
/// closing and expiring credit, in minutes.
type Day = (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64);

fn shape(result: &DailyCreditResult) -> Day {
    let offset = (result.date() - MONDAY).get_days().into();
    assert_eq!(result.nominal_target(), minutes(420));
    let mins = |duration: SignedDuration| duration.as_mins();
    (
        offset,
        mins(result.gross_credited_work()),
        mins(result.opening_balance()),
        mins(result.applied_credit()),
        mins(result.effective_target()),
        mins(result.remaining_work()),
        mins(result.new_overtime()),
        mins(result.earned_transferable_credit()),
        mins(result.closing_balance()),
        mins(result.credit_expiring_after_friday()),
    )
}

fn shapes(results: &[DailyCreditResult]) -> Vec<Day> {
    results.iter().map(shape).collect()
}

#[test]
fn unreviewed_work_remains_live_overtime_but_cannot_fund_later_days() {
    let results = calculate_week(
        &[
            DailyCreditInput {
                work_awaiting_review: minutes(540),
                ..DailyCreditInput::new(MONDAY, minutes(960))
            },
            day(1, 360),
        ],
        &settings(),
    )
    .unwrap();

    assert_eq!(results[0].new_overtime(), minutes(540));
    assert_eq!(results[0].remaining_work(), SignedDuration::ZERO);
    assert_eq!(
        results[0].earned_transferable_credit(),
        SignedDuration::ZERO
    );
    assert_eq!(results[1].applied_credit(), SignedDuration::ZERO);
    assert_eq!(results[1].remaining_work(), minutes(60));
}

#[test]
fn reviewed_surplus_still_transfers_when_another_part_of_the_day_awaits_review() {
    let results = calculate_week(
        &[
            DailyCreditInput {
                work_awaiting_review: minutes(540),
                ..DailyCreditInput::new(MONDAY, minutes(1080))
            },
            day(1, 360),
        ],
        &settings(),
    )
    .unwrap();

    assert_eq!(results[0].earned_transferable_credit(), minutes(120));
    assert_eq!(results[1].applied_credit(), minutes(60));
    assert_eq!(results[1].closing_balance(), minutes(60));
}

#[test]
fn rejects_review_work_outside_gross_work() {
    for review in [-1, 421] {
        let result = calculate_week(
            &[DailyCreditInput {
                work_awaiting_review: minutes(review),
                ..DailyCreditInput::new(MONDAY, minutes(420))
            }],
            &settings(),
        );

        assert_eq!(
            result,
            Err(InvalidWeeklyCredit::AwaitingReviewOutOfRange),
            "{review}"
        );
    }
}

#[test]
fn applies_monday_overtime_to_tuesday_without_creating_new_overtime_at_the_effective_target() {
    let results = calculate_week(&[day(0, 450), day(1, 390)], &settings()).unwrap();

    assert_eq!(
        shapes(&results),
        [
            (0, 450, 0, 0, 420, 0, 30, 30, 30, 0),
            (1, 390, 30, 30, 390, 0, 0, 0, 0, 0),
        ]
    );
}

#[test]
fn applies_only_enough_credit_to_reach_the_floor_and_expires_the_friday_balance_prospectively() {
    let results = calculate_week(
        &[
            day(0, 600),
            day(1, 400),
            day(2, 400),
            day(3, 400),
            day(4, 400),
        ],
        &settings(),
    )
    .unwrap();

    assert_eq!(
        shapes(&results),
        [
            (0, 600, 0, 0, 420, 0, 180, 180, 180, 0),
            (1, 400, 180, 60, 360, 0, 40, 40, 160, 0),
            (2, 400, 160, 60, 360, 0, 40, 40, 140, 0),
            (3, 400, 140, 60, 360, 0, 40, 40, 120, 0),
            (4, 400, 120, 60, 360, 0, 40, 40, 100, 100),
        ]
    );
}

#[test]
fn disabled_credit_keeps_factual_overtime_without_making_it_transferable() {
    let results = calculate_week(
        &[day(0, 450), day(1, 390)],
        &WorkTimeSettings {
            weekly_credit_enabled: false,
            ..settings()
        },
    )
    .unwrap();

    assert_eq!(
        shapes(&results),
        [
            (0, 450, 0, 0, 420, 0, 30, 0, 0, 0),
            (1, 390, 0, 0, 420, 30, 0, 0, 0, 0),
        ]
    );
}

#[test]
fn a_separate_week_starts_at_zero_after_friday_credit_expires() {
    let friday = calculate_week(
        &[
            day(0, 600),
            day(1, 400),
            day(2, 400),
            day(3, 400),
            day(4, 400),
        ],
        &settings(),
    )
    .unwrap()[4];
    let next_monday = calculate_week(&[day(7, 420)], &settings()).unwrap()[0];

    assert_eq!(friday.credit_expiring_after_friday(), minutes(100));
    assert_eq!(next_monday.opening_balance(), SignedDuration::ZERO);
    assert_eq!(next_monday.applied_credit(), SignedDuration::ZERO);
    assert_eq!(next_monday.effective_target(), minutes(420));
}

#[test]
fn applies_credit_before_an_incomplete_day_without_creating_debt() {
    let tuesday = calculate_week(&[day(0, 600), day(1, 120)], &settings()).unwrap()[1];

    assert_eq!(shape(&tuesday), (1, 120, 180, 60, 360, 240, 0, 0, 120, 0));
}

#[test]
fn preserves_ticks_at_target_and_floor_boundaries() {
    let results = calculate_week(
        &[
            DailyCreditInput::new(MONDAY, minutes(420) + TICK),
            DailyCreditInput::new(day_of_week(1), minutes(420) - TICK),
        ],
        &settings(),
    )
    .unwrap();

    assert_eq!(results[0].new_overtime(), TICK);
    assert_eq!(results[0].earned_transferable_credit(), TICK);
    assert_eq!(results[1].applied_credit(), TICK);
    assert_eq!(results[1].effective_target(), minutes(420) - TICK);
    assert_eq!(results[1].remaining_work(), SignedDuration::ZERO);
    assert_eq!(results[1].new_overtime(), SignedDuration::ZERO);
    assert_eq!(results[1].closing_balance(), SignedDuration::ZERO);

    let equal_floor = calculate_week(
        &[day(0, 480), day(1, 480)],
        &WorkTimeSettings {
            minimum_effective_target: minutes(420),
            ..settings()
        },
    )
    .unwrap();

    assert_eq!(equal_floor[1].applied_credit(), SignedDuration::ZERO);
    assert_eq!(equal_floor[1].effective_target(), minutes(420));
    assert_eq!(equal_floor[1].closing_balance(), minutes(120));
}

#[test]
fn rejects_invalid_weekly_prefixes_and_negative_work() {
    let reject = |inputs: &[DailyCreditInput]| calculate_week(inputs, &settings()).unwrap_err();

    assert_eq!(reject(&[]), InvalidWeeklyCredit::WeekLength);
    assert_eq!(
        reject(&[day(1, 420)]),
        InvalidWeeklyCredit::NotStartingOnMonday
    );
    assert_eq!(
        reject(&[day(0, 420), day(2, 420)]),
        InvalidWeeklyCredit::NotConsecutive
    );
    assert_eq!(
        reject(&[day(0, 420), day(0, 420)]),
        InvalidWeeklyCredit::NotConsecutive
    );
    assert_eq!(
        reject(&[day(0, 420), day(2, 420), day(1, 420)]),
        InvalidWeeklyCredit::NotConsecutive
    );
    assert_eq!(
        reject(&[
            day(0, 420),
            day(1, 420),
            day(2, 420),
            day(3, 420),
            day(4, 420),
            day(5, 420)
        ]),
        InvalidWeeklyCredit::WeekLength
    );
    assert_eq!(
        reject(&[DailyCreditInput::new(MONDAY, -TICK)]),
        InvalidWeeklyCredit::NegativeGrossWork
    );
}

#[test]
fn rejects_invalid_credit_targets_at_the_policy_boundary() {
    let reject =
        |settings: WorkTimeSettings| calculate_week(&[day(0, 420)], &settings).unwrap_err();

    assert_eq!(
        reject(WorkTimeSettings {
            nominal_daily_target: SignedDuration::ZERO,
            ..settings()
        }),
        InvalidWeeklyCredit::NominalTargetOutOfRange
    );
    assert_eq!(
        reject(WorkTimeSettings {
            nominal_daily_target: SignedDuration::from_hours(24) + TICK,
            ..settings()
        }),
        InvalidWeeklyCredit::NominalTargetOutOfRange
    );
    assert_eq!(
        reject(WorkTimeSettings {
            minimum_effective_target: SignedDuration::ZERO,
            ..settings()
        }),
        InvalidWeeklyCredit::MinimumTargetOutOfRange
    );
    assert_eq!(
        reject(WorkTimeSettings {
            minimum_effective_target: minutes(421),
            ..settings()
        }),
        InvalidWeeklyCredit::MinimumTargetOutOfRange
    );
}
