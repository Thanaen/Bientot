//! Replays `tests/fixtures/snapshot.txt`, which the .NET `DailySummarySnapshotFactory` wrote
//! before the .NET app was removed, and requires the same stored snapshot and the same input
//! fingerprint, byte for byte, on every line. The days are the generated accounting inputs of
//! the domain fixtures, read with the domain's own notation.

#[path = "../../bientot-domain/tests/accounting_case/mod.rs"]
mod accounting_case;

use accounting_case::{
    AccountingCase, at, days_after, duration, flag, interval, minutes, opt, rejected, relative,
    replay, time_of_day, unescape,
};
use bientot_core::snapshot;
use bientot_domain::configuration::Settings;
use bientot_domain::credit::{DailyCreditInput, calculate_week};
use bientot_domain::history::DailySummarySnapshot;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::{
    SourceDiagnosticInterval, SourceDiagnostics, SourceFreshness, SourceHealth, SourceIntervalKind,
    SourceQuality,
};
use bientot_domain::summaries::{DailyAccountingCalculation, build_with_credit};
use jiff::Timestamp;

#[test]
fn daily_summary_snapshots_match_dotnet() {
    replay("snapshot", include_str!("fixtures/snapshot.txt"), |input| {
        let columns: Vec<_> = input.split('|').collect();
        let case = AccountingCase::parse(&columns[..12]);
        let result = case.calculate()?;
        let origin = case.origin;
        let now = at(origin, case.now);
        let source_id = unescape(columns[15]);
        let mut diagnostics = columns[16].split(';');
        let health: Vec<_> = diagnostics.next().unwrap().split(',').collect();
        let freshness = match health[0] {
            "Fresh" => SourceFreshness::Fresh,
            "Stale" => SourceFreshness::Stale,
            _ => SourceFreshness::Unavailable,
        };
        let quality = match health[1] {
            "Complete" => SourceQuality::Complete,
            "Incomplete" => SourceQuality::Incomplete,
            "Ambiguous" => SourceQuality::Ambiguous,
            "Malformed" => SourceQuality::Malformed,
            _ => SourceQuality::Unauthorized,
        };
        let degraded = freshness != SourceFreshness::Fresh || quality != SourceQuality::Complete;
        let intervals = diagnostics.map(|text| {
            let f: Vec<_> = text.split(',').collect();
            let kind = match f[0] {
                "PendingSettlement" => SourceIntervalKind::PendingSettlement,
                "InteriorCompatibilitySeam" => SourceIntervalKind::InteriorCompatibilitySeam,
                _ => SourceIntervalKind::UnresolvedCoverage,
            };
            SourceDiagnosticInterval::new(interval(origin, f[1], f[2]).unwrap(), kind, &source_id)
                .unwrap()
        });
        let source = SourceDiagnostics::new(
            SourceHealth {
                source_id: source_id.clone(),
                freshness,
                quality,
                last_successful_at: Some(now),
                degraded_reason: degraded.then(|| "degraded".to_owned()),
            },
            UtcInterval::new(origin, at(origin, case.window_end)).unwrap(),
            now,
            intervals,
        )
        .map_err(rejected)?;
        let calculation = DailyAccountingCalculation::new(
            result,
            duration(columns[12]),
            case.lunch().unwrap(),
            at(origin, columns[13]),
            duration(columns[14]),
            source,
        )
        .map_err(rejected)?;

        let credit: Vec<_> = columns[17].split(',').collect();
        let times: Vec<_> = columns[19].split(',').collect();
        let settlement: Vec<_> = columns[21].split(',').collect();
        let mut settings = Settings {
            nominal_daily_target: duration(columns[12]),
            minimum_effective_target: duration(credit[0]),
            weekly_credit_enabled: flag(credit[1]),
            weekly_debt_enabled: flag(credit[2]),
            maximum_weekly_debt: opt(credit[3]).map(duration),
            lunch_start: time_of_day(times[0]),
            lunch_end: time_of_day(times[1]),
            long_lock_review_threshold: duration(times[2]),
            preferred_latest_departure: time_of_day(times[3]),
            orange_warning_margin: duration(columns[14]),
            ..Settings::default()
        };
        settings.activity_watch.selected_bucket_id = opt(columns[20]).map(unescape);
        settings.activity_watch.settlement.afk_timeout = duration(settlement[0]);
        settings.activity_watch.settlement.poll_interval = duration(settlement[1]);
        settings.activity_watch.settlement.maximum_jitter = duration(settlement[2]);

        let monday = days_after(
            case.day,
            -i64::from(case.day.weekday().to_monday_zero_offset()),
        );
        let week: Vec<_> = columns[18]
            .split('/')
            .enumerate()
            .map(|(index, day)| {
                let f: Vec<_> = day.split(':').collect();
                DailyCreditInput {
                    date: days_after(monday, index as i64),
                    gross_credited_work: duration(f[0]),
                    work_awaiting_review: duration(f[1]),
                    is_finished: flag(f[2]),
                }
            })
            .collect();
        let credit = *calculate_week(&week, &settings)
            .map_err(rejected)?
            .last()
            .unwrap();
        let summary = build_with_credit(&calculation, credit).map_err(rejected)?;
        let flags: Vec<_> = columns[22].split(',').collect();
        let snapshot = snapshot::create(
            &calculation,
            credit,
            &summary,
            &settings,
            flag(flags[0]),
            flag(flags[1]),
        );
        Ok(render(&snapshot, origin))
    });
}

fn render(snapshot: &DailySummarySnapshot, origin: Timestamp) -> String {
    let totals = [
        snapshot.credited_work(),
        snapshot.excluded_work(),
        snapshot.unknown_work(),
        snapshot.nominal_target(),
        snapshot.effective_target(),
        snapshot.carry_available(),
        snapshot.carry_applied(),
        snapshot.carry_earned(),
        snapshot.carry_expired(),
        snapshot.carry_debt(),
    ]
    .map(minutes)
    .join(",");
    let breakdown = snapshot
        .breakdown()
        .entries()
        .iter()
        .map(|entry| {
            [
                format!("{:?}", entry.disposition()),
                format!("{:?}", entry.label()),
                minutes(entry.duration()),
                entry.interval().map_or("_".to_owned(), |interval| {
                    relative(interval.start(), origin)
                }),
                entry
                    .interval()
                    .map_or("_".to_owned(), |interval| relative(interval.end(), origin)),
                entry
                    .adjustment_id()
                    .map_or("_".to_owned(), |id| id.as_str().to_owned()),
                entry
                    .review_id()
                    .map_or("_".to_owned(), |id| id.as_str().to_owned()),
            ]
            .join(",")
        })
        .collect::<Vec<_>>()
        .join(";");
    [
        snapshot.input_fingerprint().to_owned(),
        format!("{totals},{}", u8::from(snapshot.debt_pending())),
        snapshot
            .projected_departure()
            .map_or("_".to_owned(), |departure| relative(departure, origin)),
        format!("{:?}", snapshot.indicator_state()),
        format!("{:?}", snapshot.source_quality()),
        u8::from(snapshot.has_pending_review()).to_string(),
        breakdown,
    ]
    .join("|")
}
