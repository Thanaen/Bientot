use jiff::civil::{Date, date};

use super::*;
use crate::accounting::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AdjustmentId, AutomaticOrigin, ManualDurationCredit, WorkdayEventHistory, calculate,
};
use crate::configuration::WorkTimeSettings;
use crate::credit::{DailyCreditInput, calculate_week};
use crate::forecasting::assess;
use crate::intervals::UtcInterval;
use crate::sources::{SourceFreshness, SourceQuality};
use crate::time::WorkdayWindow;

const DAY: Date = date(2026, 8, 14);

fn utc(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn computed_at() -> Timestamp {
    utc(DAY, 15, 0)
}

fn hours(hours: i64) -> SignedDuration {
    SignedDuration::from_hours(hours)
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn accounting_on(day: Date, credited_work: SignedDuration) -> AccountingResult {
    let start = utc(day, 8, 0);
    let end = start + credited_work;
    let bounds = UtcInterval::new(start, end).unwrap();
    let input = AccountingInput::new(
        WorkdayWindow {
            workday: LocalWorkday::new(day, "UTC"),
            start,
            end,
        },
        end,
        AccountingPolicy::new(
            UtcInterval::new(utc(day, 12, 0), utc(day, 14, 0)).unwrap(),
            minutes(90),
        )
        .unwrap(),
        ActivityCoverage::new(
            bounds,
            [ActivitySegment::new(
                bounds,
                ActivityState::Active,
                AutomaticOrigin::new("activitywatch").unwrap(),
            )],
        ),
        WorkdayEventHistory::default(),
        vec![],
    )
    .unwrap();
    calculate(&input).unwrap()
}

fn diagnostics(accounting: &AccountingResult, health: SourceHealth) -> SourceDiagnostics {
    let timeline = accounting.timeline();
    SourceDiagnostics::new(
        health,
        UtcInterval::new(timeline.window().start, timeline.through()).unwrap(),
        computed_at(),
        [],
    )
    .unwrap()
}

fn healthy() -> SourceHealth {
    SourceHealth::healthy("activitywatch", computed_at())
}

fn projection(is_visible: bool) -> DepartureProjection {
    DepartureProjection {
        projected_departure: utc(DAY, 17, 30),
        is_visible,
    }
}

fn parts(
    accounting: AccountingResult,
    effective_target: SignedDuration,
    indicator_state: IndicatorState,
    departure_projection: Option<DepartureProjection>,
    health: SourceHealth,
) -> DailySummaryParts {
    let source_diagnostics = diagnostics(&accounting, health);
    DailySummaryParts {
        computed_at: computed_at(),
        assessment: assess(&accounting, &source_diagnostics),
        accounting,
        nominal_target: hours(7),
        effective_target,
        departure_projection,
        indicator_state,
        source_diagnostics,
        credit_result: None,
    }
}

fn create(
    credited_work: SignedDuration,
    effective_target: SignedDuration,
    state: IndicatorState,
) -> Result<DailySummary, InvalidSummary> {
    let projection = (state != IndicatorState::Blue).then(|| projection(true));
    DailySummary::new(parts(
        accounting_on(DAY, credited_work),
        effective_target,
        state,
        projection,
        healthy(),
    ))
}

#[test]
fn incomplete_summary_exposes_remaining_work() {
    let summary = create(hours(5), hours(7), IndicatorState::Green).unwrap();

    assert!(!summary.is_complete());
    assert_eq!(summary.remaining_work(), hours(2));
    assert_eq!(summary.credited_overtime(), SignedDuration::ZERO);
}

#[test]
fn complete_summary_exposes_credited_overtime() {
    let summary = create(minutes(450), hours(7), IndicatorState::Blue).unwrap();

    assert!(summary.is_complete());
    assert_eq!(summary.remaining_work(), SignedDuration::ZERO);
    assert_eq!(summary.credited_overtime(), minutes(30));
}

#[test]
fn summary_derives_credited_work_from_timeline_and_manual_duration_credit_exactly_once() {
    let timeline_accounting = accounting_on(DAY, hours(2));
    let accounting = AccountingResult::new(
        timeline_accounting.timeline().clone(),
        vec![ManualDurationCredit::new(
            AdjustmentId::new("manual-duration-1").unwrap(),
            minutes(30),
        )],
        timeline_accounting.pending_reviews().to_vec(),
    );

    let summary = DailySummary::new(parts(
        accounting.clone(),
        hours(7),
        IndicatorState::Green,
        Some(projection(true)),
        healthy(),
    ))
    .unwrap();

    assert_eq!(summary.accounting(), &accounting);
    assert_eq!(summary.credited_work(), minutes(150));
}

/// The .NET summary takes its workday separately and rejects one that differs from the
/// accounting's; here the workday is the accounting's, so the mismatch cannot be expressed.
#[test]
fn summary_workday_is_the_accounting_workday() {
    let other_day = DAY.tomorrow().unwrap();

    let summary = DailySummary::new(parts(
        accounting_on(other_day, hours(2)),
        hours(7),
        IndicatorState::Green,
        Some(projection(true)),
        healthy(),
    ))
    .unwrap();

    assert_eq!(summary.workday(), &LocalWorkday::new(other_day, "UTC"));
}

#[test]
fn incomplete_summary_preserves_departure_projection_visibility() {
    for is_visible in [false, true] {
        let projection = DepartureProjection {
            projected_departure: utc(DAY, 17, 30),
            is_visible,
        };

        let summary = DailySummary::new(parts(
            accounting_on(DAY, hours(5)),
            hours(7),
            IndicatorState::Green,
            Some(projection),
            healthy(),
        ))
        .unwrap();

        assert_eq!(summary.departure_projection(), Some(projection));
    }
}

#[test]
fn incomplete_summary_requires_departure_projection() {
    let result = DailySummary::new(parts(
        accounting_on(DAY, hours(5)),
        hours(7),
        IndicatorState::Green,
        None,
        healthy(),
    ));

    assert_eq!(result, Err(InvalidSummary::ProjectionContradictsCompletion));
}

#[test]
fn complete_summary_rejects_departure_projection() {
    let result = DailySummary::new(parts(
        accounting_on(DAY, hours(7)),
        hours(7),
        IndicatorState::Blue,
        Some(projection(true)),
        healthy(),
    ));

    assert_eq!(result, Err(InvalidSummary::ProjectionContradictsCompletion));
}

#[test]
fn indicator_must_agree_with_effective_target() {
    for (credited, state) in [(6, IndicatorState::Blue), (7, IndicatorState::Green)] {
        assert_eq!(
            create(hours(credited), hours(7), state),
            Err(InvalidSummary::IndicatorContradictsCompletion),
            "{credited}h {state:?}"
        );
    }
}

#[test]
fn summary_preserves_source_operation_without_withholding_resolved_accounting() {
    let health = SourceHealth::degraded(
        "activitywatch",
        SourceFreshness::Stale,
        SourceQuality::Incomplete,
        "Last refresh timed out.",
        Some(utc(DAY, 14, 55)),
    )
    .unwrap();

    let summary = DailySummary::new(parts(
        accounting_on(DAY, hours(5)),
        hours(7),
        IndicatorState::Green,
        Some(projection(true)),
        health.clone(),
    ))
    .unwrap();

    assert_eq!(summary.source_diagnostics().operation(), &health);
    assert!(summary.has_source_warning());
    assert_eq!(
        summary.assessment().forecast(),
        ForecastAvailability::Confirmed
    );
}

#[test]
fn credit_result_must_match_every_duplicated_summary_value() {
    let monday = date(2026, 9, 7);
    let tuesday = monday.tomorrow().unwrap();
    let accounting = accounting_on(tuesday, minutes(390));
    let settings = WorkTimeSettings {
        nominal_daily_target: hours(7),
        minimum_effective_target: hours(6),
        weekly_credit_enabled: true,
        ..WorkTimeSettings::default()
    };
    let tuesday_after = |monday_work, tuesday_work, settings: &WorkTimeSettings| {
        calculate_week(
            &[
                DailyCreditInput::new(monday, monday_work),
                DailyCreditInput::new(tuesday, tuesday_work),
            ],
            settings,
        )
        .unwrap()[1]
    };
    let matching = tuesday_after(hours(8), minutes(390), &settings);
    let wrong_date =
        calculate_week(&[DailyCreditInput::new(monday, minutes(390))], &settings).unwrap()[0];
    let wrong_gross = tuesday_after(hours(8), hours(6), &settings);
    let wrong_nominal = tuesday_after(
        hours(9),
        minutes(390),
        &WorkTimeSettings {
            nominal_daily_target: hours(8),
            minimum_effective_target: hours(7),
            ..settings.clone()
        },
    );
    let wrong_effective_and_applied = tuesday_after(
        hours(8),
        minutes(390),
        &WorkTimeSettings {
            minimum_effective_target: minutes(390),
            ..settings.clone()
        },
    );
    let credit_summary = |effective_target, credit_result| {
        let mut parts = parts(
            accounting.clone(),
            effective_target,
            IndicatorState::Blue,
            None,
            healthy(),
        );
        parts.credit_result = Some(credit_result);
        DailySummary::new(parts)
    };

    assert_eq!(
        credit_summary(hours(6), matching).unwrap().credit_result(),
        Some(&matching)
    );
    for mismatch in [
        wrong_date,
        wrong_gross,
        wrong_nominal,
        wrong_effective_and_applied,
    ] {
        assert_eq!(
            credit_summary(hours(6), mismatch),
            Err(InvalidSummary::CreditResultMismatch),
            "{mismatch:?}"
        );
    }
    assert_eq!(
        credit_summary(minutes(390), matching),
        Err(InvalidSummary::CreditResultMismatch)
    );
}
