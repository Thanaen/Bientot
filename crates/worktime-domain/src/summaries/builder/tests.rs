use jiff::civil::{Date, date};
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::accounting::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AutomaticOrigin, WorkdayEventHistory, calculate,
};
use crate::configuration::WorkTimeSettings;
use crate::credit::{DailyCreditInput, calculate_week};
use crate::intervals::UtcInterval;
use crate::sources::{SourceDiagnostics, SourceHealth};
use crate::time::{LocalWorkday, WorkdayWindow};

const MONDAY: Date = date(2026, 9, 7);
const TUESDAY: Date = date(2026, 9, 8);

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn settings() -> WorkTimeSettings {
    WorkTimeSettings {
        nominal_daily_target: minutes(420),
        minimum_effective_target: minutes(360),
        weekly_credit_enabled: true,
        ..WorkTimeSettings::default()
    }
}

fn utc(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn calculation(day: Date, credited_work: SignedDuration) -> DailyAccountingCalculation {
    let start = utc(day, 8, 0);
    let through = start + SignedDuration::from_hours(7);
    let credited_end = start + credited_work;
    let origin = AutomaticOrigin::new("activitywatch:test").unwrap();
    let lunch = interval(utc(day, 12, 0), utc(day, 14, 0));
    let input = AccountingInput::new(
        WorkdayWindow {
            workday: LocalWorkday::new(day, "UTC"),
            start,
            end: start + SignedDuration::from_hours(16),
        },
        through,
        AccountingPolicy::new(lunch, minutes(90)).unwrap(),
        ActivityCoverage::new(
            interval(start, through),
            [ActivitySegment::new(
                interval(start, credited_end),
                ActivityState::Active,
                origin.clone(),
            )]
            .into_iter()
            .chain(
                UtcInterval::new(credited_end, through)
                    .ok()
                    .map(|idle| ActivitySegment::new(idle, ActivityState::Afk, origin)),
            ),
        ),
        WorkdayEventHistory::default(),
        vec![],
    )
    .unwrap();
    let accounting = calculate(&input).unwrap();
    let diagnostics = SourceDiagnostics::new(
        SourceHealth::healthy("activitywatch", through),
        interval(start, through),
        through,
        [],
    )
    .unwrap();
    DailyAccountingCalculation::new(
        accounting,
        settings().nominal_daily_target,
        lunch,
        utc(day, 18, 30),
        settings().orange_warning_margin,
        diagnostics,
    )
    .unwrap()
}

fn tuesday_credit(
    monday_work: SignedDuration,
    tuesday_work: SignedDuration,
    settings: &WorkTimeSettings,
) -> DailyCreditResult {
    calculate_week(
        &[
            DailyCreditInput::new(MONDAY, monday_work),
            DailyCreditInput::new(TUESDAY, tuesday_work),
        ],
        settings,
    )
    .unwrap()[1]
}

#[test]
fn credit_effective_target_controls_completion_projection_and_overtime_together() {
    let calculation = calculation(TUESDAY, minutes(405));
    let credit = tuesday_credit(minutes(450), minutes(405), &settings());

    let nominal = build_nominal(&calculation).unwrap();
    let credited = build_with_credit(&calculation, credit).unwrap();

    assert_eq!(nominal.effective_target(), minutes(420));
    assert_eq!(nominal.credit_result(), None);
    assert_eq!(nominal.remaining_work(), minutes(15));
    assert!(nominal.departure_projection().is_some());
    assert!(!nominal.is_complete());

    assert_eq!(credited.effective_target(), minutes(390));
    assert_eq!(credited.applied_carry_credit(), minutes(30));
    assert_eq!(credited.credit_result(), Some(&credit));
    assert_eq!(credited.remaining_work(), SignedDuration::ZERO);
    assert_eq!(credited.credited_overtime(), minutes(15));
    assert_eq!(credited.departure_projection(), None);
    assert_eq!(credited.indicator_state(), IndicatorState::Blue);
    assert!(credited.is_complete());
    assert_eq!(credited.accounting(), calculation.accounting());
    assert_eq!(
        credited.source_diagnostics(),
        calculation.source_diagnostics()
    );
    assert_eq!(credited.computed_at(), calculation.observed_at());
}

#[test]
fn applied_credit_moves_an_incomplete_projection_by_the_target_reduction() {
    let calculation = calculation(TUESDAY, minutes(360));
    let credit = tuesday_credit(minutes(450), minutes(360), &settings());

    let nominal = build_nominal(&calculation).unwrap();
    let credited = build_with_credit(&calculation, credit).unwrap();

    assert_eq!(nominal.remaining_work(), minutes(60));
    assert_eq!(
        nominal.departure_projection().unwrap().projected_departure,
        utc(TUESDAY, 16, 0)
    );
    assert_eq!(credited.remaining_work(), minutes(30));
    assert_eq!(
        credited.departure_projection().unwrap().projected_departure,
        utc(TUESDAY, 15, 30)
    );
}

#[test]
fn credit_must_match_the_calculation_date_gross_work_and_nominal_target() {
    let calculation = calculation(TUESDAY, minutes(405));
    let wrong_date =
        calculate_week(&[DailyCreditInput::new(MONDAY, minutes(390))], &settings()).unwrap()[0];
    let wrong_gross = tuesday_credit(minutes(450), minutes(360), &settings());
    let wrong_nominal = tuesday_credit(
        minutes(480),
        minutes(405),
        &WorkTimeSettings {
            nominal_daily_target: minutes(480),
            minimum_effective_target: minutes(420),
            ..settings()
        },
    );

    assert_eq!(
        build_with_credit(&calculation, wrong_date),
        Err(InvalidSummary::CreditForAnotherDay)
    );
    assert_eq!(
        build_with_credit(&calculation, wrong_gross),
        Err(InvalidSummary::CreditForOtherWork)
    );
    assert_eq!(
        build_with_credit(&calculation, wrong_nominal),
        Err(InvalidSummary::CreditForAnotherTarget)
    );
}

#[test]
fn a_weekend_day_shows_blue_once_its_target_is_reached_and_green_before() {
    let saturday = date(2026, 9, 12);

    assert_eq!(
        build_nominal(&calculation(saturday, minutes(420)))
            .unwrap()
            .indicator_state(),
        IndicatorState::Blue
    );
    assert_eq!(
        build_nominal(&calculation(saturday, minutes(300)))
            .unwrap()
            .indicator_state(),
        IndicatorState::Green
    );
}
