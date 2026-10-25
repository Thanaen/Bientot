//! Replays the differential fixtures that the .NET domain wrote before the .NET app was removed,
//! and requires the Rust domain to produce the same output on every line. Each fixture file
//! describes its columns in its header; the notation is shared:
//!
//! - a line is `input => expected`;
//! - instants and durations are minutes with an optional tick remainder (`615`, `615+1`, `-5-3`),
//!   instants relative to the origin the file names;
//! - a rejected input is `!ExceptionType(paramName)` or `!Adjustment:<error>`, so each Rust error
//!   maps to the exception .NET throws for it (see [`Rejection`]).

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use worktime_domain::accounting::{AccountedOrigin, AccountingResult, SegmentKind};
use worktime_domain::adjustments::validate_for_append;
use worktime_domain::configuration::{NotificationRule, NotificationSettings, WorkTimeSettings};
use worktime_domain::credit::{DailyCreditInput, build_ledger, calculate_week};
use worktime_domain::forecasting::{indicator_state, overtime, project_departure};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::notifications::{NotificationPolicyInput, eligible};
use worktime_domain::sources::{
    SourceDiagnosticInterval, SourceDiagnostics, SourceFreshness, SourceHealth, SourceIntervalKind,
    SourceQuality,
};
use worktime_domain::summaries::{
    DailyAccountingCalculation, DailySummary, IndicatorState, build_nominal, build_with_credit,
};
use worktime_domain::time::{
    LocalWorkday, TimeZoneProvider, UnknownTimeZone, WorkdayCalendar, WorkdayWindow,
};

mod accounting_case;

use accounting_case::*;

/// Since #11 (D-010), a credit of the user's may lie over a long-lock confirmation, which .NET
/// refused: the confirmation is valid, or refused for what else is wrong with it.
const CREDIT_OVER_CONFIRMATION: [Revision; 4] = [
    Revision {
        line: 101,
        net: "|!Adjustment:OverlappingActiveAdjustments",
        now: "|ok",
    },
    Revision {
        line: 673,
        net: "|!Adjustment:OverlappingActiveAdjustments",
        now: "|ok",
    },
    Revision {
        line: 678,
        net: "|!Adjustment:OverlappingActiveAdjustments",
        now: "|!Adjustment:UnknownLongLockReview",
    },
    Revision {
        line: 743,
        net: "|!Adjustment:OverlappingActiveAdjustments",
        now: "|ok",
    },
];

#[test]
fn accounting_matches_dotnet() {
    replay_revised(
        "accounting",
        include_str!("fixtures/accounting.txt"),
        &CREDIT_OVER_CONFIRMATION,
        |input| {
            let columns: Vec<_> = input.split('|').collect();
            let case = AccountingCase::parse(&columns[..12]);
            let result = case.calculate()?;
            let verdict = match opt(columns[12]) {
                None => "_".to_owned(),
                Some(candidate) => case
                    .adjustment(candidate)
                    .and_then(|candidate| {
                        validate_for_append(&result, &case.adjustments().unwrap(), &candidate)
                            .map_err(rejected)
                    })
                    .map_or_else(|error| error, |()| "ok".to_owned()),
            };
            Ok(format!(
                "{}|{verdict}",
                render_accounting(&result, case.origin)
            ))
        },
    );
}

#[test]
fn daily_summary_matches_dotnet() {
    replay("summary", include_str!("fixtures/summary.txt"), |input| {
        let columns: Vec<_> = input.split('|').collect();
        let case = AccountingCase::parse(&columns[..12]);
        let result = case.calculate()?;
        let origin = case.origin;
        let now = at(origin, case.now);
        let mut diagnostics = columns[15].split(';');
        let health: Vec<_> = diagnostics.next().unwrap().split(',').collect();
        let freshness = match health[0] {
            "Fresh" => SourceFreshness::Fresh,
            "Stale" => SourceFreshness::Stale,
            _ => SourceFreshness::Unavailable,
        };
        let quality = match health[1] {
            "Complete" => SourceQuality::Complete,
            "Incomplete" => SourceQuality::Incomplete,
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
            SourceDiagnosticInterval::new(interval(origin, f[1], f[2]).unwrap(), kind, "aw")
                .unwrap()
        });
        let source = SourceDiagnostics::new(
            SourceHealth {
                source_id: "aw".to_owned(),
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
        let summary = match columns[16].split(',').collect::<Vec<_>>()[..] {
            ["N"] => build_nominal(&calculation),
            [
                "C",
                nominal,
                minimum,
                credit,
                debt,
                maximum_debt,
                pick,
                days,
            ] => {
                let settings = WorkTimeSettings {
                    nominal_daily_target: duration(nominal),
                    minimum_effective_target: duration(minimum),
                    weekly_credit_enabled: flag(credit),
                    weekly_debt_enabled: flag(debt),
                    maximum_weekly_debt: opt(maximum_debt).map(duration),
                    ..WorkTimeSettings::default()
                };
                let monday = days_after(
                    case.day,
                    -i64::from(case.day.weekday().to_monday_zero_offset()),
                );
                let inputs: Vec<_> = days
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
                let results = calculate_week(&inputs, &settings).map_err(rejected)?;
                build_with_credit(&calculation, results[pick.parse::<usize>().unwrap()])
            }
            _ => panic!("unknown summary mode {}", columns[16]),
        }
        .map_err(rejected)?;
        // #164: .NET rejects a weekend day that reaches its target; Rust shows it blue.
        if !calculation.workday().is_working_day() && summary.is_complete() {
            assert_eq!(summary.indicator_state(), IndicatorState::Blue);
            return Err("!ArgumentException(indicatorState)".to_owned());
        }
        Ok(render_summary(&summary, origin))
    });
}

#[test]
fn weekly_credit_and_ledger_match_dotnet() {
    let monday = date("2026-09-07");
    let origin = midnight(monday);
    replay("credit", include_str!("fixtures/credit.txt"), |input| {
        let columns: Vec<_> = input.split('|').collect();
        let s: Vec<_> = columns[0].split(',').collect();
        let settings = WorkTimeSettings {
            nominal_daily_target: duration(s[0]),
            minimum_effective_target: duration(s[1]),
            weekly_credit_enabled: flag(s[2]),
            weekly_debt_enabled: flag(s[3]),
            maximum_weekly_debt: opt(s[4]).map(duration),
            ..WorkTimeSettings::default()
        };
        let inputs: Vec<_> = items(columns[1])
            .map(|day| {
                let f: Vec<_> = day.split(':').collect();
                DailyCreditInput {
                    date: days_after(monday, f[0].parse().unwrap()),
                    gross_credited_work: duration(f[1]),
                    work_awaiting_review: duration(f[2]),
                    is_finished: flag(f[3]),
                }
            })
            .collect();
        let results = calculate_week(&inputs, &settings).map_err(rejected)?;
        let days = results
            .iter()
            .map(|r| {
                [
                    r.opening_balance(),
                    r.applied_credit(),
                    r.earned_transferable_credit(),
                    r.opening_debt(),
                    r.repaid_debt(),
                    r.incurred_debt(),
                    r.effective_target(),
                    r.remaining_work(),
                    r.new_overtime(),
                    r.closing_balance(),
                    r.closing_debt(),
                    r.net_closing_balance(),
                    r.credit_expiring_after_friday(),
                ]
                .map(minutes)
                .join(",")
            })
            .collect::<Vec<_>>()
            .join(";");
        let revisions = |text: &str| -> BTreeMap<Date, i64> {
            items(text)
                .map(|pair| {
                    let (day, revision) = pair.split_once(':').unwrap();
                    (
                        days_after(monday, day.parse().unwrap()),
                        revision.parse().unwrap(),
                    )
                })
                .collect()
        };
        let friday: Vec<_> = columns[3].split(',').collect();
        let window = WorkdayWindow {
            workday: LocalWorkday::new(
                days_after(monday, friday[0].parse().unwrap()),
                "Europe/Paris",
            ),
            start: at(origin, friday[1]),
            end: at(origin, friday[2]),
        };
        let ledgers = match build_ledger(
            &results,
            &revisions(columns[2]),
            &window,
            at(origin, columns[4]),
            &[],
        ) {
            Err(error) => format!("{}|_", rejected(error)),
            Ok(first) => {
                let second = build_ledger(
                    &results,
                    &revisions(columns[5]),
                    &window,
                    at(origin, columns[6]),
                    &first,
                )
                .map(|entries| {
                    entries
                        .iter()
                        .map(|entry| {
                            let id = first
                                .iter()
                                .position(|known| known.id() == entry.id())
                                .map_or_else(
                                    || entry.id().as_str().to_owned(),
                                    |k| format!("#{k}"),
                                );
                            format!("{id}@{}", relative(entry.created_at(), origin))
                        })
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .unwrap_or_else(rejected);
                let ids: Vec<_> = first.iter().map(|entry| entry.id().as_str()).collect();
                format!("{}|{second}", ids.join(";"))
            }
        };
        Ok(format!("{days}|{ledgers}"))
    });
}

#[test]
fn forecasts_match_dotnet() {
    let origin = midnight(date("2026-09-07"));
    replay("forecast", include_str!("fixtures/forecast.txt"), |input| {
        let (day, indicator_input) = input.split_once('|').unwrap();
        let f: Vec<_> = day.split(',').collect();
        let (now, credited, target) = (at(origin, f[0]), duration(f[1]), duration(f[2]));
        // .NET takes the lunch bounds as two instants and names the end when they are reversed.
        let projection = UtcInterval::new(at(origin, f[3]), at(origin, f[4]))
            .map_err(|_| "!ArgumentOutOfRangeException(lunchEnd)".to_owned())
            .and_then(|lunch| project_departure(now, credited, target, lunch).map_err(rejected));
        let i: Vec<_> = indicator_input.split(',').collect();
        let projected = match projection {
            Ok(Some(projection)) if flag(i[2]) => Some(projection.projected_departure),
            _ => None,
        };
        let indicator = indicator_state(
            credited,
            target,
            projected,
            at(origin, i[0]),
            duration(i[1]),
        )
        .map_or_else(rejected, |state| format!("{state:?}"));
        let overtime = overtime(credited, target).map_or_else(rejected, minutes);
        let projection = projection.map_or_else(
            |error| error,
            |projection| {
                projection.map_or("_".to_owned(), |p| {
                    format!(
                        "{},{}",
                        relative(p.projected_departure, origin),
                        bit(p.lunch_over)
                    )
                })
            },
        );
        Ok(format!("{projection}|{indicator}|{overtime}"))
    });
}

#[test]
fn notifications_match_dotnet() {
    let monday = date("2026-09-07");
    let origin = midnight(monday);
    replay(
        "notifications",
        include_str!("fixtures/notifications.txt"),
        |input| {
            let f: Vec<_> = input.split('|').collect();
            let rule = |text: &str| {
                let (enabled, lead) = text.split_once(',').unwrap();
                NotificationRule::new(flag(enabled), duration(lead))
            };
            let triggers = eligible(&NotificationPolicyInput {
                workday: LocalWorkday::new(days_after(monday, f[0].parse().unwrap()), "UTC"),
                observed_at: at(origin, f[1]),
                preferred_latest_departure: at(origin, f[2]),
                remaining_work: duration(f[3]),
                is_complete: flag(f[4]),
                is_healthy: flag(f[5]),
                settings: NotificationSettings {
                    completion_approach: rule(f[6]),
                    latest_departure_approach: rule(f[7]),
                    day_complete: rule(f[8]),
                },
            });
            Ok(if triggers.is_empty() {
                "none".to_owned()
            } else {
                triggers
                    .iter()
                    .map(|t| format!("{t:?}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
        },
    );
}

#[test]
fn settings_validation_matches_dotnet() {
    replay("settings", include_str!("fixtures/settings.txt"), |input| {
        let (kind, values) = input.split_once('|').unwrap();
        if kind == "U" {
            let mut settings = WorkTimeSettings::default();
            settings.activity_watch.base_url = unescape(values);
            let usable = !settings
                .validate()
                .iter()
                .any(|error| error.resource_key() == "SettingsSource");
            return Ok(bit(usable).to_owned());
        }
        let f: Vec<_> = values.split(',').collect();
        let rule = |enabled: &str, lead: &str| NotificationRule::new(flag(enabled), duration(lead));
        let mut settings = WorkTimeSettings {
            schema_version: f[0].parse().unwrap(),
            nominal_daily_target: duration(f[1]),
            minimum_effective_target: duration(f[2]),
            lunch_start: time_of_day(f[3]),
            lunch_end: time_of_day(f[4]),
            preferred_latest_departure: time_of_day(f[5]),
            orange_warning_margin: duration(f[6]),
            long_lock_review_threshold: duration(f[7]),
            maximum_weekly_debt: opt(f[8]).map(duration),
            language_override: opt(f[9]).map(|text| text.trim_matches('\'').to_owned()),
            notifications: NotificationSettings {
                completion_approach: rule(f[15], f[16]),
                latest_departure_approach: rule(f[17], f[18]),
                day_complete: rule(f[19], f[20]),
            },
            ..WorkTimeSettings::default()
        };
        let source = &mut settings.activity_watch;
        source.request_timeout = duration(f[10]);
        source.settlement.afk_timeout = duration(f[11]);
        source.settlement.poll_interval = duration(f[12]);
        source.settlement.maximum_jitter = duration(f[13]);
        source.base_url = unescape(f[23]);
        settings.built_in_activity.idle_threshold = duration(f[14]);
        settings.logging.maximum_file_count = f[21].parse().unwrap();
        settings.logging.maximum_file_bytes = f[22].parse().unwrap();
        let errors: Vec<_> = settings
            .validate()
            .iter()
            .map(|error| error.resource_key())
            .collect();
        Ok(if errors.is_empty() {
            "none".to_owned()
        } else {
            errors.join(",")
        })
    });
}

#[test]
fn workday_calendar_matches_dotnet() {
    struct Zones(String);

    impl TimeZoneProvider for Zones {
        fn local(&self) -> (String, TimeZone) {
            (self.0.clone(), self.get(&self.0).unwrap())
        }

        fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
            TimeZone::get(id).map_err(|_| UnknownTimeZone(id.to_owned()))
        }
    }

    replay("calendar", include_str!("fixtures/calendar.txt"), |input| {
        let f: Vec<_> = input.split('|').collect();
        let calendar = WorkdayCalendar::new(Zones(f[1].to_owned()));
        let day = date(f[2]);
        let workday = LocalWorkday::new(day, f[1]);
        let origin = midnight(day);
        Ok(match f[0] {
            "I" => relative(
                calendar.instant(&workday, time_of_day(f[3])).unwrap(),
                origin,
            ),
            "N" => {
                let (start, end) = f[3].split_once(',').unwrap();
                let interval = calendar
                    .interval(&workday, time_of_day(start), time_of_day(end))
                    .map_err(rejected)?;
                format!(
                    "{},{}",
                    relative(interval.start(), origin),
                    relative(interval.end(), origin)
                )
            }
            "W" => {
                let window = calendar.window(&workday).unwrap();
                format!(
                    "{},{}",
                    relative(window.start, origin),
                    relative(window.end, origin)
                )
            }
            "D" => calendar.workday(at(origin, f[3])).date.to_string(),
            kind => panic!("unknown calendar case {kind}"),
        })
    });
}

fn render_accounting(result: &AccountingResult, origin: Timestamp) -> String {
    let span = |interval: UtcInterval, separator: &str| {
        format!(
            "{}{separator}{}",
            relative(interval.start(), origin),
            relative(interval.end(), origin)
        )
    };
    let timeline = result.timeline();
    let segments = timeline
        .segments()
        .iter()
        .map(|segment| {
            let (kind, review) = match segment.kind() {
                SegmentKind::Credited => ("C", String::new()),
                SegmentKind::ProvisionalCredit { review_id } => ("P", format!(",{review_id}")),
                SegmentKind::Excluded => ("X", String::new()),
                SegmentKind::Unknown => ("U", String::new()),
            };
            format!(
                "{kind},{:?},{},{}{review}",
                segment.label(),
                span(segment.interval(), ","),
                render_origin(segment.origin())
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let totals = [
        relative(timeline.through(), origin),
        minutes(timeline.credited_work()),
        minutes(timeline.confirmed_credited_work()),
        minutes(timeline.provisional_work()),
        minutes(timeline.excluded_work()),
        minutes(timeline.unknown_work()),
    ]
    .join(",");
    let coverage = timeline
        .credited_coverage()
        .iter()
        .map(|interval| span(*interval, ","))
        .collect::<Vec<_>>()
        .join(";");
    let credits = result
        .manual_duration_credits()
        .iter()
        .map(|credit| format!("{},{}", credit.adjustment_id(), minutes(credit.duration())))
        .collect::<Vec<_>>()
        .join(";");
    let reviews = result
        .pending_reviews()
        .iter()
        .map(|review| {
            let coverage = review
                .provisional_coverage
                .iter()
                .map(|interval| span(*interval, ".."))
                .collect::<Vec<_>>()
                .join("/");
            format!(
                "{},{},{},{coverage}",
                review.id,
                span(review.locked_episode, ","),
                bit(review.is_open)
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    format!(
        "{segments}|{totals}|{},{}|{coverage}|{credits}|{reviews}",
        minutes(result.credited_work()),
        minutes(result.work_awaiting_review())
    )
}

fn render_origin(origin: &AccountedOrigin) -> String {
    match origin {
        AccountedOrigin::Automatic(automatic) => automatic.source_id().to_owned(),
        AccountedOrigin::Manual {
            adjustment_id,
            replaces,
        } => format!("{adjustment_id}>{}", render_origin(replaces)),
    }
}

fn render_summary(summary: &DailySummary, origin: Timestamp) -> String {
    let projection = summary.departure_projection().map_or("_".to_owned(), |p| {
        format!(
            "{},{}",
            relative(p.projected_departure, origin),
            bit(p.lunch_over)
        )
    });
    let assessment = summary.assessment();
    let reasons: Vec<_> = std::iter::once(format!("{:?}", assessment.forecast()))
        .chain(
            assessment
                .reasons()
                .iter()
                .map(|reason| format!("{reason:?}")),
        )
        .collect();
    let flags = [
        summary.can_show_forecast(),
        summary.has_source_diagnostics(),
        summary.has_source_warning(),
        summary.is_complete(),
    ]
    .map(bit)
    .join(",");
    format!(
        "{},{}|{projection}|{:?}|{}|{flags}|{},{},{}",
        minutes(summary.effective_target()),
        minutes(summary.applied_carry_credit()),
        summary.indicator_state(),
        reasons.join(";"),
        minutes(summary.remaining_work()),
        minutes(summary.credited_overtime()),
        minutes(summary.credited_work())
    )
}
