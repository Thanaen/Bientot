//! Properties of the weekly credit policy and its lot ledger that hold for any valid week.

use std::collections::{BTreeMap, HashSet};

use jiff::civil::{Date, date};
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::credit::{
    CreditLedgerEntry, CreditLedgerEntryKind, CreditOrigin, DailyCreditInput, DailyCreditResult,
    build_ledger, calculate_week,
};
use worktime_domain::time::{LocalWorkday, WorkdayWindow};

const MONDAY: Date = date(2026, 9, 7);

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn day(offset: i64) -> Date {
    MONDAY.checked_add(jiff::Span::new().days(offset)).unwrap()
}

#[derive(Clone, Debug)]
struct Week {
    nominal: i64,
    floor_gap: i64,
    credit: bool,
    debt: bool,
    maximum_debt: Option<i64>,
    days: Vec<(i64, i64, bool)>,
}

impl Week {
    fn settings(&self) -> WorkTimeSettings {
        WorkTimeSettings {
            nominal_daily_target: minutes(self.nominal),
            minimum_effective_target: minutes(self.nominal - self.floor_gap),
            weekly_credit_enabled: self.credit,
            weekly_debt_enabled: self.debt,
            maximum_weekly_debt: self.maximum_debt.map(minutes),
            ..WorkTimeSettings::default()
        }
    }

    fn inputs(&self) -> Vec<DailyCreditInput> {
        (0..)
            .zip(&self.days)
            .map(
                |(offset, &(gross, awaiting, is_finished))| DailyCreditInput {
                    work_awaiting_review: minutes(awaiting.min(gross)),
                    is_finished,
                    ..DailyCreditInput::new(day(offset), minutes(gross))
                },
            )
            .collect()
    }

    fn results(&self) -> Vec<DailyCreditResult> {
        calculate_week(&self.inputs(), &self.settings()).unwrap()
    }
}

fn week() -> impl Strategy<Value = Week> {
    (
        60i64..600,
        0i64..120,
        any::<bool>(),
        any::<bool>(),
        prop::option::of(1i64..300),
        prop::collection::vec((0i64..900, 0i64..300, any::<bool>()), 1..=5),
    )
        .prop_map(|(nominal, gap, credit, debt, maximum_debt, days)| Week {
            nominal,
            floor_gap: gap.min(nominal - 1),
            credit,
            debt,
            maximum_debt,
            days,
        })
}

fn friday_window() -> WorkdayWindow {
    WorkdayWindow {
        workday: LocalWorkday::new(day(4), "Europe/Paris"),
        start: "2026-09-11T06:00:00Z".parse().unwrap(),
        end: "2026-09-11T20:00:00Z".parse().unwrap(),
    }
}

fn total(entries: &[CreditLedgerEntry], kind: CreditLedgerEntryKind) -> SignedDuration {
    entries
        .iter()
        .filter(|entry| entry.kind() == kind)
        .map(CreditLedgerEntry::amount)
        .sum()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn balances_chain_from_day_to_day_and_close_on_the_sum_of_daily_deltas(week in week()) {
        let results = week.results();

        let mut balance = SignedDuration::ZERO;
        let mut debt = SignedDuration::ZERO;
        for result in &results {
            prop_assert_eq!(result.opening_balance(), balance);
            prop_assert_eq!(result.opening_debt(), debt);
            balance = result.closing_balance();
            debt = result.closing_debt();
        }
        let earned: SignedDuration = results.iter().map(DailyCreditResult::earned_transferable_credit).sum();
        let applied: SignedDuration = results.iter().map(DailyCreditResult::applied_credit).sum();
        prop_assert_eq!(balance, earned - applied);
        prop_assert_eq!(results.last().unwrap().net_closing_balance(), balance - debt);
    }

    #[test]
    fn targets_stay_between_the_floor_and_nominal_and_no_amount_is_negative(week in week()) {
        let settings = week.settings();

        for result in week.results() {
            prop_assert!(result.effective_target() >= settings.minimum_effective_target);
            prop_assert!(result.effective_target() <= settings.nominal_daily_target);
            prop_assert!(result.applied_credit() <= result.opening_balance());
            for amount in [
                result.applied_credit(),
                result.earned_transferable_credit(),
                result.closing_balance(),
                result.new_overtime(),
                result.remaining_work(),
                result.repaid_debt(),
                result.incurred_debt(),
                result.closing_debt(),
            ] {
                prop_assert!(!amount.is_negative(), "{result:?}");
            }
            prop_assert!(result.new_overtime().is_zero() || result.remaining_work().is_zero());
            prop_assert!(result.repaid_debt() <= result.opening_debt());
        }
    }

    #[test]
    fn debt_exists_only_when_enabled_and_never_exceeds_its_limit(week in week()) {
        for result in week.results() {
            if !(week.credit && week.debt) {
                prop_assert_eq!(result.closing_debt(), SignedDuration::ZERO);
            }
            if let Some(maximum) = week.maximum_debt {
                prop_assert!(result.closing_debt() <= minutes(maximum));
            }
        }
    }

    #[test]
    fn the_ledger_conserves_every_lot_and_matches_the_daily_results(
        week in week(),
        revisions in prop::collection::vec(1i64..5, 5),
        observed_minutes in 0i64..8000,
    ) {
        let results = week.results();
        let revisions: BTreeMap<_, _> = results.iter().zip(&revisions).map(|(result, &revision)| (result.date(), revision)).collect();
        let observed_at = "2026-09-07T00:00:00Z".parse::<Timestamp>().unwrap() + minutes(observed_minutes);
        let window = friday_window();

        let ledger = build_ledger(&results, &revisions, &window, observed_at, &[]).unwrap();

        let expired = results.len() == 5 && observed_at >= window.end;
        let closing = results.last().unwrap().closing_balance();
        prop_assert_eq!(
            total(&ledger, CreditLedgerEntryKind::Earned)
                - total(&ledger, CreditLedgerEntryKind::Applied)
                - total(&ledger, CreditLedgerEntryKind::Expired),
            if expired { SignedDuration::ZERO } else { closing }
        );
        for result in &results {
            let on_day = |kind| total(
                &ledger.iter().filter(|entry| entry.workday_date() == result.date()).cloned().collect::<Vec<_>>(),
                kind,
            );
            prop_assert_eq!(on_day(CreditLedgerEntryKind::Applied), result.applied_credit());
            prop_assert_eq!(on_day(CreditLedgerEntryKind::Earned), result.earned_transferable_credit());
        }
        let mut drawn: BTreeMap<(Date, i64), SignedDuration> = BTreeMap::new();
        for entry in ledger.iter().filter(|entry| entry.kind() != CreditLedgerEntryKind::Earned) {
            let CreditOrigin { workday_date, revision } = entry.origin();
            *drawn.entry((workday_date, revision)).or_default() += entry.amount();
        }
        for ((origin, revision), amount) in drawn {
            let earned = ledger
                .iter()
                .find(|entry| entry.kind() == CreditLedgerEntryKind::Earned && entry.workday_date() == origin)
                .map(CreditLedgerEntry::amount);
            prop_assert_eq!(revisions[&origin], revision);
            prop_assert!(earned.is_some_and(|earned| amount <= earned));
        }
        let ids: HashSet<_> = ledger.iter().map(CreditLedgerEntry::id).collect();
        prop_assert_eq!(ids.len(), ledger.len());
    }

    #[test]
    fn rebuilding_from_the_previous_ledger_is_idempotent(week in week(), later_minutes in 1i64..600) {
        let results = week.results();
        let revisions: BTreeMap<_, _> = results.iter().map(|result| (result.date(), 1)).collect();
        let first_observed = "2026-09-11T19:00:00Z".parse::<Timestamp>().unwrap();
        let window = friday_window();
        let first = build_ledger(&results, &revisions, &window, first_observed, &[]).unwrap();

        let rebuilt = build_ledger(&results, &revisions, &window, first_observed + minutes(later_minutes), &first).unwrap();

        let expires_later = results.len() == 5 && first_observed + minutes(later_minutes) >= window.end;
        if expires_later {
            prop_assert_eq!(&rebuilt[..first.len()], &first[..]);
        } else {
            prop_assert_eq!(rebuilt, first);
        }
    }
}
