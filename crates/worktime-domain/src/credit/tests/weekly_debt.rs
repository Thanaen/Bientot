use super::*;

fn debt_settings() -> WorkTimeSettings {
    WorkTimeSettings {
        weekly_debt_enabled: true,
        ..settings()
    }
}

fn running(offset: i64, gross_minutes: i64) -> DailyCreditInput {
    DailyCreditInput {
        is_finished: false,
        ..day(offset, gross_minutes)
    }
}

fn week(inputs: &[DailyCreditInput]) -> Vec<DailyCreditResult> {
    calculate_week(inputs, &debt_settings()).unwrap()
}

/// Opening, repaid, incurred and closing debt, earned credit and net balance, in minutes.
fn debt(result: &DailyCreditResult) -> (i64, i64, i64, i64, i64, i64) {
    (
        result.opening_debt().as_mins(),
        result.repaid_debt().as_mins(),
        result.incurred_debt().as_mins(),
        result.closing_debt().as_mins(),
        result.earned_transferable_credit().as_mins(),
        result.net_closing_balance().as_mins(),
    )
}

fn movement_minutes(entry: &CreditLedgerEntry) -> (Date, CreditLedgerEntryKind, i64, Date) {
    let (date, kind, amount, origin, _) = movement(entry);
    (date, kind, amount.as_mins(), origin)
}

#[test]
fn shortfall_becomes_debt_that_later_surplus_repays_before_earning_credit() {
    // Owner example (#73): Mon 6h, Tue 7h30, Wed 8h, Thu 7h, Friday still running.
    let results = week(&[
        day(0, 360),
        day(1, 450),
        day(2, 480),
        day(3, 420),
        running(4, 0),
    ]);

    assert_eq!(debt(&results[0]), (0, 0, 60, 60, 0, -60));
    assert_eq!(debt(&results[1]), (60, 30, 0, 30, 0, -30));
    assert_eq!(debt(&results[2]), (30, 30, 0, 0, 30, 30));
    assert_eq!(debt(&results[3]), (0, 0, 0, 0, 30, 30));
    assert_eq!(results[3].applied_credit(), minutes(30));
    assert_eq!(results[4].applied_credit(), minutes(30));
    assert_eq!(results[4].closing_debt(), SignedDuration::ZERO);

    // Debt never raises a target: Tuesday still aims for the nominal 7h.
    assert!(
        results[..3]
            .iter()
            .all(|result| result.effective_target() == minutes(420))
    );
}

#[test]
fn repaid_surplus_is_never_earned_so_the_lot_ledger_stays_valid() {
    let results = week(&[
        day(0, 360),
        day(1, 450),
        day(2, 480),
        day(3, 420),
        running(4, 0),
    ]);

    let ledger = build_ledger(
        &results,
        &revisions_of(&results, &[1; 5]),
        &friday_window(MONDAY),
        utc(day_of_week(4), 10),
        &[],
    )
    .unwrap();

    use CreditLedgerEntryKind::{Applied, Earned};
    assert_eq!(
        ledger.iter().map(movement_minutes).collect::<Vec<_>>(),
        [
            (day_of_week(2), Earned, 30, day_of_week(2)),
            (day_of_week(3), Applied, 30, day_of_week(2)),
            (day_of_week(3), Earned, 30, day_of_week(3)),
            (day_of_week(4), Applied, 30, day_of_week(3)),
        ]
    );
}

#[test]
fn debt_left_on_friday_is_reported_and_the_next_week_starts_at_zero() {
    let this_week = week(&[
        day(0, 400),
        day(1, 400),
        day(2, 420),
        day(3, 410),
        day(4, 400),
    ]);
    let next_week = week(&[day(7, 400)]);

    assert_eq!(debt(&this_week[4]), (50, 0, 20, 70, 0, -70));
    assert_eq!(
        this_week[4].credit_expiring_after_friday(),
        SignedDuration::ZERO
    );
    assert_eq!(debt(&next_week[0]), (0, 0, 20, 20, 0, -20));
}

#[test]
fn days_without_counted_work_create_no_debt() {
    let results = week(&[day(0, 0), day(1, 400), day(2, 0)]);

    assert_eq!(results[0].incurred_debt(), SignedDuration::ZERO);
    assert_eq!(results[1].incurred_debt(), minutes(20));
    assert_eq!(results[2].incurred_debt(), SignedDuration::ZERO);
    assert_eq!(results[2].closing_debt(), minutes(20));
}

#[test]
fn the_running_day_creates_no_debt_until_it_has_ended() {
    let running_day = week(&[day(0, 360), running(1, 300)]);
    let finished = week(&[day(0, 360), day(1, 300)]);

    assert_eq!(running_day[1].closing_debt(), minutes(60));
    assert_eq!(finished[1].closing_debt(), minutes(180));
}

#[test]
fn the_running_day_still_repays_debt_with_its_surplus_like_it_earns_credit() {
    let results = week(&[day(0, 360), running(1, 450)]);

    assert_eq!(debt(&results[1]), (60, 30, 0, 30, 0, -30));
}

#[test]
fn the_cap_keeps_debt_at_the_limit_and_drops_the_excess() {
    let results = calculate_week(
        &[
            day(0, 360),
            day(1, 360),
            day(2, 480),
            day(3, 300),
            day(4, 480),
        ],
        &WorkTimeSettings {
            maximum_weekly_debt: Some(minutes(90)),
            ..debt_settings()
        },
    )
    .unwrap();

    assert_eq!(
        results.iter().map(debt).collect::<Vec<_>>(),
        [
            (0, 0, 60, 60, 0, -60),
            (60, 0, 30, 90, 0, -90),
            (90, 60, 0, 30, 0, -30),
            (30, 0, 60, 90, 0, -90),
            (90, 60, 0, 30, 0, -30),
        ]
    );
}

#[test]
fn work_awaiting_review_avoids_debt_but_repays_nothing_until_confirmed() {
    let awaiting = |offset, gross, review| DailyCreditInput {
        work_awaiting_review: minutes(review),
        ..day(offset, gross)
    };
    let results = week(&[day(0, 360), awaiting(1, 480, 60), awaiting(2, 400, 100)]);
    let confirmed = week(&[day(0, 360), day(1, 480), day(2, 400)]);

    assert_eq!(debt(&results[1]), (60, 0, 0, 60, 0, -60));
    assert_eq!(debt(&results[2]), (60, 0, 20, 80, 0, -80));
    assert_eq!(debt(&confirmed[1]), (60, 60, 0, 0, 0, 0));
    assert_eq!(debt(&confirmed[2]), (0, 0, 20, 20, 0, -20));
}

#[test]
fn correcting_a_past_day_recalculates_the_debt_of_later_days() {
    let before = week(&[day(0, 360), day(1, 480), day(2, 420)]);
    let corrected = week(&[day(0, 420), day(1, 480), day(2, 420)]);

    assert_eq!(debt(&before[1]), (60, 60, 0, 0, 0, 0));
    assert_eq!(before[2].applied_credit(), SignedDuration::ZERO);
    assert_eq!(debt(&corrected[0]), (0, 0, 0, 0, 0, 0));
    assert_eq!(debt(&corrected[1]), (0, 0, 0, 0, 60, 60));
    assert_eq!(corrected[2].applied_credit(), minutes(60));
}

#[test]
fn a_shortfall_sits_beside_leftover_credit_instead_of_consuming_it() {
    // Decision: debt is never taken from earned lots. Leftover credit keeps lowering later
    // targets (up to the floor) while the debt waits for surplus; the net balance is exact.
    let results = week(&[day(0, 600), day(1, 300), day(2, 420)]);

    assert_eq!(results[1].effective_target(), minutes(360));
    assert_eq!(results[1].closing_balance(), minutes(120));
    assert_eq!(debt(&results[1]), (0, 0, 60, 60, 0, 60));
    assert_eq!(results[2].applied_credit(), minutes(60));
    assert_eq!(debt(&results[2]), (60, 60, 0, 0, 0, 60));

    let ledger = build_ledger(
        &results,
        &revisions_of(&results, &[1; 3]),
        &friday_window(MONDAY),
        utc(day_of_week(2), 20),
        &[],
    )
    .unwrap();
    assert_eq!(sum(&ledger, CreditLedgerEntryKind::Applied), minutes(120));
}

#[test]
fn without_debt_the_week_matches_the_credit_only_policy() {
    for (debt_enabled, credit_enabled) in [(false, true), (true, false)] {
        let inputs = [
            day(0, 360),
            day(1, 480),
            day(2, 300),
            day(3, 0),
            day(4, 450),
        ];
        let baseline = calculate_week(
            &inputs,
            &WorkTimeSettings {
                weekly_debt_enabled: false,
                weekly_credit_enabled: credit_enabled,
                ..debt_settings()
            },
        )
        .unwrap();

        let results = calculate_week(
            &inputs,
            &WorkTimeSettings {
                weekly_debt_enabled: debt_enabled,
                weekly_credit_enabled: credit_enabled,
                ..debt_settings()
            },
        )
        .unwrap();

        assert_eq!(baseline, results);
        for result in &results {
            assert_eq!(result.closing_debt(), SignedDuration::ZERO);
            assert_eq!(result.closing_balance(), result.net_closing_balance());
        }
        if credit_enabled {
            assert_eq!(results[1].earned_transferable_credit(), minutes(60));
            assert_eq!(results[2].applied_credit(), minutes(60));
        }
    }
}
