//! Ported from `DailySummarySnapshotFactoryTests.cs`. `tests/dotnet_snapshots.rs` replays
//! fingerprints exported from .NET over generated days.

use bientot_domain::accounting::{
    self, AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AutomaticOrigin, LongLockReviewId, WorkdayEventHistory,
};
use bientot_domain::credit::{DailyCreditInput, calculate_week};
use bientot_domain::intervals::{IntervalSet, UtcInterval};
use bientot_domain::sources::{SourceFreshness, SourceHealth};
use bientot_domain::summaries::build_with_credit;
use bientot_domain::time::{FixedTimeZoneProvider, LocalWorkday, WorkdayCalendar};
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;

use super::*;

const MONDAY: Date = date(2026, 9, 7);

fn instant(hour: i8) -> Timestamp {
    MONDAY
        .at(hour, 0, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

/// The arguments of the .NET `CreateSnapshot` helper, defaulted the same way.
struct Case {
    observed_at: Timestamp,
    freshness: SourceFreshness,
    last_successful_at: Option<Timestamp>,
    source_id: &'static str,
    settings: Settings,
    reviews: Option<Vec<LongLockReview>>,
    resolved_lunch_shift: SignedDuration,
    resolved_departure_shift: SignedDuration,
}

impl Default for Case {
    fn default() -> Self {
        Self {
            observed_at: instant(20),
            freshness: SourceFreshness::Fresh,
            last_successful_at: None,
            source_id: "activitywatch:test",
            settings: Settings::default(),
            reviews: None,
            resolved_lunch_shift: SignedDuration::ZERO,
            resolved_departure_shift: SignedDuration::ZERO,
        }
    }
}

/// Monday in UTC, active from 08:00 to 15:00 and observed through 18:00.
fn snapshot(case: Case) -> DailySummarySnapshot {
    let settings = &case.settings;
    let workday = LocalWorkday::new(MONDAY, "UTC");
    let calendar = WorkdayCalendar::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC));
    let window = calendar.window(&workday).unwrap();
    let through = instant(18);
    let observed = UtcInterval::new(window.start, through).unwrap();
    let source = AutomaticOrigin::new(case.source_id).unwrap();
    let input = AccountingInput::new(
        window.clone(),
        through,
        AccountingPolicy::new(
            calendar
                .interval(&workday, settings.lunch_start, settings.lunch_end)
                .unwrap(),
            settings.long_lock_review_threshold,
        )
        .unwrap(),
        ActivityCoverage::new(
            observed,
            [ActivitySegment::new(
                UtcInterval::new(instant(8), instant(15)).unwrap(),
                ActivityState::Active,
                source,
            )],
        ),
        WorkdayEventHistory::default(),
        Vec::new(),
    )
    .unwrap();
    let mut result = accounting::calculate(&input).unwrap();
    if let Some(reviews) = case.reviews {
        result = result.with_pending_reviews(reviews);
    }
    let diagnostics = SourceDiagnostics::new(
        SourceHealth {
            source_id: case.source_id.to_owned(),
            freshness: case.freshness,
            quality: SourceQuality::Complete,
            last_successful_at: case.last_successful_at,
            degraded_reason: None,
        },
        observed,
        case.observed_at,
        [],
    )
    .unwrap();
    let lunch_start = calendar.instant(&workday, settings.lunch_start).unwrap();
    let lunch_end = calendar.instant(&workday, settings.lunch_end).unwrap();
    let credited_work = result.credited_work();
    let calculation = DailyAccountingCalculation::new(
        result,
        settings.nominal_daily_target,
        UtcInterval::new(
            lunch_start + case.resolved_lunch_shift,
            lunch_end + case.resolved_lunch_shift,
        )
        .unwrap(),
        calendar
            .instant(&workday, settings.preferred_latest_departure)
            .unwrap()
            + case.resolved_departure_shift,
        settings.orange_warning_margin,
        diagnostics,
    )
    .unwrap();
    let [credit] =
        calculate_week(&[DailyCreditInput::new(MONDAY, credited_work)], settings).unwrap()[..]
    else {
        panic!("one day in, one day out");
    };
    let summary = build_with_credit(&calculation, credit).unwrap();
    create(&calculation, credit, &summary, settings, false, false)
}

fn review(id: &str, start_hour: i8, is_open: bool) -> LongLockReview {
    let episode = UtcInterval::new(instant(start_hour), instant(start_hour + 1)).unwrap();
    LongLockReview {
        id: LongLockReviewId::new(id).unwrap(),
        locked_episode: episode,
        provisional_coverage: IntervalSet::new([episode]),
        reason: LongLockReviewReason::ThresholdExceeded,
        is_open,
    }
}

/// .NET formats dates with the current culture unless told otherwise, so its test runs under a
/// Thai calendar. Rust has no ambient culture; the fingerprint's date text is checked here instead.
#[test]
fn fingerprint_is_invariant_across_current_cultures() {
    assert_eq!(format_date(MONDAY), "2026-09-07");
    assert_eq!(
        snapshot(Case::default()).input_fingerprint(),
        snapshot(Case::default()).input_fingerprint()
    );
}

#[test]
fn operational_observation_metadata_does_not_change_material_fingerprint() {
    let first = snapshot(Case {
        observed_at: instant(20),
        freshness: SourceFreshness::Fresh,
        last_successful_at: Some(instant(19)),
        ..Case::default()
    });
    let second = snapshot(Case {
        observed_at: instant(21),
        freshness: SourceFreshness::Stale,
        last_successful_at: Some(instant(18)),
        ..Case::default()
    });

    assert_ne!(first.computed_at(), second.computed_at());
    assert_eq!(first.input_fingerprint(), second.input_fingerprint());
    assert_eq!(first.workday(), second.workday());
    assert_eq!(first.credited_work(), second.credited_work());
    assert_eq!(first.excluded_work(), second.excluded_work());
    assert_eq!(first.unknown_work(), second.unknown_work());
    assert_eq!(first.nominal_target(), second.nominal_target());
    assert_eq!(first.effective_target(), second.effective_target());
    assert_eq!(first.carry_available(), second.carry_available());
    assert_eq!(first.carry_applied(), second.carry_applied());
    assert_eq!(first.carry_earned(), second.carry_earned());
    assert_eq!(first.carry_expired(), second.carry_expired());
    assert_eq!(first.projected_departure(), second.projected_departure());
    assert_eq!(first.indicator_state(), second.indicator_state());
    assert_eq!(first.source_quality(), second.source_quality());
    assert_eq!(first.has_pending_review(), second.has_pending_review());
    assert_eq!(first.breakdown(), second.breakdown());
}

#[test]
fn canonical_review_ordering_is_stable_and_resolved_inputs_and_source_identity_are_material() {
    let earlier = review("duplicate", 9, true);
    let later = review("duplicate", 10, false);
    let both = || Some(vec![earlier.clone(), later.clone()]);
    let forward = snapshot(Case {
        reviews: both(),
        ..Case::default()
    });
    let reversed = snapshot(Case {
        reviews: Some(vec![later.clone(), earlier.clone()]),
        ..Case::default()
    });

    assert_eq!(forward.input_fingerprint(), reversed.input_fingerprint());
    assert_ne!(
        forward.input_fingerprint(),
        snapshot(Case {
            source_id: "activitywatch:other",
            reviews: both(),
            ..Case::default()
        })
        .input_fingerprint()
    );
    assert_ne!(
        forward.input_fingerprint(),
        snapshot(Case {
            reviews: both(),
            resolved_lunch_shift: SignedDuration::from_mins(30),
            ..Case::default()
        })
        .input_fingerprint()
    );
    assert_ne!(
        forward.input_fingerprint(),
        snapshot(Case {
            reviews: both(),
            resolved_departure_shift: SignedDuration::from_mins(30),
            ..Case::default()
        })
        .input_fingerprint()
    );
}

#[test]
fn fingerprint_with_debt_off_keeps_the_format_from_before_debt_existed() {
    // Golden value of the .NET suite, computed before weekly debt existed: turning debt off must
    // not re-revise history.
    const BEFORE_DEBT: &str = "daily-summary-material:v1:sha256:db46169579d84f25807baba590f8086e516b4807c186a0e9ef129440acc28542";

    assert_eq!(snapshot(Case::default()).input_fingerprint(), BEFORE_DEBT);
    assert_eq!(
        snapshot(Case {
            settings: Settings {
                maximum_weekly_debt: Some(SignedDuration::from_hours(2)),
                ..Settings::default()
            },
            ..Case::default()
        })
        .input_fingerprint(),
        BEFORE_DEBT
    );
    assert_ne!(
        snapshot(Case {
            settings: Settings {
                weekly_debt_enabled: true,
                ..Settings::default()
            },
            ..Case::default()
        })
        .input_fingerprint(),
        BEFORE_DEBT
    );
}
