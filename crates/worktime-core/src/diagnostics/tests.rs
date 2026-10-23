//! Ported from `DiagnosticSnapshotFactoryTests.cs`. The export is tested through the public API
//! in `tests/diagnostics_export.rs`.

use jiff::civil::{Date, date};
use worktime_domain::accounting::{
    self, AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AutomaticOrigin, WorkdayEventHistory,
};
use worktime_domain::forecasting::assess;
use worktime_domain::intervals::UtcInterval;
use worktime_domain::sources::{SourceDiagnostics, SourceHealth};
use worktime_domain::summaries::{DailySummary, DailySummaryParts};
use worktime_domain::time::{LocalWorkday, WorkdayWindow};

use super::*;
use crate::accounting::ActivityReadDisposition;
use crate::test_support::utc;
use crate::weekly::SourceEvidenceUnavailable;

const DATE: Date = date(2026, 9, 10);

fn computed_at() -> Timestamp {
    utc(2026, 9, 10, 16)
}

fn runtime() -> DiagnosticRuntimeSnapshot {
    let version = |major, minor, build, revision| DiagnosticNumericVersion {
        major,
        minor,
        build,
        revision,
    };
    DiagnosticRuntimeSnapshot {
        application_version: version(1, 2, 3, 4),
        operating_system_version: version(10, 0, 26200, 0),
        dot_net_version: version(10, 0, 11, 0),
        process_architecture: DiagnosticArchitecture::X64,
        operating_system_architecture: DiagnosticArchitecture::X64,
    }
}

/// Active for eight hours from 08:00 UTC and complete against its six-hour effective target,
/// as the .NET `AccountingFixture.Active` day.
fn summary() -> DailySummary {
    let workday = LocalWorkday::new(DATE, "UTC");
    let start = utc(2026, 9, 10, 8);
    let end = computed_at();
    let bounds = UtcInterval::new(start, end).unwrap();
    let accounting = accounting::calculate(
        &AccountingInput::new(
            WorkdayWindow {
                workday,
                start,
                end,
            },
            end,
            AccountingPolicy::new(
                UtcInterval::new(utc(2026, 9, 10, 12), utc(2026, 9, 10, 14)).unwrap(),
                SignedDuration::from_mins(90),
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
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let diagnostics = SourceDiagnostics::new(
        SourceHealth::healthy("activitywatch", computed_at()),
        bounds,
        computed_at(),
        [],
    )
    .unwrap();
    let assessment = assess(&accounting, &diagnostics);
    DailySummary::new(DailySummaryParts {
        computed_at: computed_at(),
        accounting,
        nominal_target: SignedDuration::from_hours(7),
        effective_target: SignedDuration::from_hours(6),
        departure_projection: None,
        indicator_state: IndicatorState::Blue,
        source_diagnostics: diagnostics,
        assessment,
        credit_result: None,
    })
    .unwrap()
}

fn current(summary: &DailySummary) -> WorkdayRefreshState {
    WorkdayRefreshState {
        summary: Some(summary.clone()),
        source_failure: None,
    }
}

#[test]
fn maps_only_safe_settings_and_useful_current_aggregates() {
    let mut settings = WorkTimeSettings {
        nominal_daily_target: SignedDuration::from_mins(450),
        minimum_effective_target: SignedDuration::from_mins(390),
        lunch_start: Time::constant(12, 15, 0, 0),
        lunch_end: Time::constant(13, 45, 0, 0),
        preferred_latest_departure: Time::constant(18, 20, 0, 0),
        language_override: Some("SENTINEL-language".to_owned()),
        ..WorkTimeSettings::default()
    };
    settings.activity_watch.base_url = "https://SENTINEL-host/private".to_owned();
    settings.activity_watch.selected_bucket_id = Some("SENTINEL-bucket".to_owned());
    settings.activity_watch.api_key_reference = Some("SENTINEL-key-reference".to_owned());
    settings.updates.last_checked_at = Some("2026-09-09T01:02:03Z".parse().unwrap());
    let summary = summary();

    let snapshot = create_snapshot(
        DiagnosticInitializationState::Ready,
        Some(&settings),
        Some(&current(&summary)),
        Some(DATE),
        computed_at(),
        runtime(),
    );

    assert_eq!(
        snapshot.initialization,
        DiagnosticInitializationState::Ready
    );
    let settings = snapshot.settings.unwrap();
    assert_eq!(settings.nominal_daily_target_ticks, 270_000_000_000);
    assert_eq!(settings.lunch_start_minute_of_day, 735);
    assert_eq!(settings.lunch_end_minute_of_day, 825);
    assert_eq!(settings.preferred_latest_departure_minute_of_day, 1100);
    assert_eq!(settings.activity_watch.request_timeout_ticks, 50_000_000);
    assert_eq!(settings.logging.maximum_file_bytes, 1_048_576);
    assert!(settings.check_for_updates);
    let projected = snapshot.summary.unwrap();
    assert_eq!(
        projected.availability,
        DiagnosticSummaryAvailability::Current
    );
    assert_eq!(
        projected.credited_work_ticks,
        ticks(summary.credited_work())
    );
    assert_eq!(
        projected.remaining_work_ticks,
        ticks(summary.remaining_work())
    );
    assert_eq!(
        projected.credited_overtime_ticks,
        ticks(summary.credited_overtime())
    );
    assert_eq!(projected.indicator, DiagnosticIndicator::Blue);
    assert_eq!(projected.source_freshness, DiagnosticSourceFreshness::Fresh);
}

#[test]
fn preserves_retained_aggregates_but_marks_refresh_failure_stale() {
    let summary = summary();
    let failure = SourceEvidenceUnavailable {
        workday_date: DATE,
        diagnostics: summary.source_diagnostics().clone(),
        disposition: ActivityReadDisposition::Unavailable,
    };

    let snapshot = create_snapshot(
        DiagnosticInitializationState::Failed,
        None,
        Some(&WorkdayRefreshState {
            summary: Some(summary.clone()),
            source_failure: Some(failure),
        }),
        Some(DATE),
        computed_at(),
        runtime(),
    );

    assert_eq!(
        snapshot.initialization,
        DiagnosticInitializationState::Failed
    );
    assert_eq!(snapshot.settings, None);
    let projected = snapshot.summary.unwrap();
    assert_eq!(projected.availability, DiagnosticSummaryAvailability::Stale);
    assert_eq!(
        projected.credited_work_ticks,
        ticks(summary.credited_work())
    );
}

#[test]
fn does_not_substitute_settings_or_summary_before_initialization() {
    let snapshot = create_snapshot(
        DiagnosticInitializationState::Starting,
        None,
        None,
        None,
        computed_at(),
        runtime(),
    );

    assert_eq!(snapshot.settings, None);
    assert_eq!(snapshot.summary, None);
}

#[test]
fn uses_clock_age_and_workday_to_avoid_claiming_old_state_is_current() {
    let summary = summary();
    for (age_minutes, different_workday) in [(3, false), (0, true)] {
        let workday = if different_workday {
            DATE.tomorrow().unwrap()
        } else {
            DATE
        };
        let snapshot = create_snapshot(
            DiagnosticInitializationState::Ready,
            Some(&WorkTimeSettings::default()),
            Some(&current(&summary)),
            Some(workday),
            computed_at() + SignedDuration::from_mins(age_minutes),
            runtime(),
        );

        assert_eq!(
            snapshot.summary.unwrap().availability,
            DiagnosticSummaryAvailability::Stale,
            "{age_minutes} minutes old, different workday: {different_workday}"
        );
    }
}
