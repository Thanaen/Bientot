//! The accounting inputs of `fixtures/accounting.txt` and the notation every fixture shares,
//! replayed by `dotnet_fixtures.rs` and, through a `#[path]` module, by the daily summary snapshot
//! fixtures of `worktime-core`, which build on the same generated days.

// Each replay uses part of the notation.
#![allow(dead_code)]

use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp};
use worktime_domain::accounting::{
    self, AccountingInput, AccountingPolicy, AccountingResult, ActivityCoverage, ActivitySegment,
    ActivityState, AdjustmentId, AutomaticOrigin, InvalidAccountingInput, LongLockReviewId,
    NonPositiveReviewThreshold, PowerState, Recorded, RecordedTransition, SessionState,
    TrackingState, WorkdayEventHistory, WorkdayEventId, WorkdayStateAnchors,
};
use worktime_domain::adjustments::{
    AdjustmentKind, AdjustmentValidationError, InvalidAdjustment, ManualAdjustment,
    ManualClassification,
};
use worktime_domain::credit::{InvalidLedgerInput, InvalidWeeklyCredit};
use worktime_domain::forecasting::InvalidForecastInput;
use worktime_domain::intervals::{EmptyInterval, UtcInterval};
use worktime_domain::sources::InvalidDiagnostics;
use worktime_domain::summaries::{InvalidCalculation, InvalidSummary};
use worktime_domain::time::{CalendarError, LocalWorkday, WorkdayWindow};

/// One accounting input, in the column order of `accounting.txt`.
pub struct AccountingCase<'a> {
    pub origin: Timestamp,
    pub day: Date,
    pub zone: &'a str,
    pub window_end: &'a str,
    pub now: &'a str,
    pub columns: &'a [&'a str],
}

impl<'a> AccountingCase<'a> {
    pub fn parse(columns: &'a [&'a str]) -> Self {
        Self {
            origin: columns[0].parse().unwrap(),
            day: date(columns[1]),
            zone: columns[2],
            window_end: columns[3],
            now: columns[4],
            columns,
        }
    }

    pub fn lunch(&self) -> Result<UtcInterval, String> {
        let (start, end) = self.columns[5].split_once(',').unwrap();
        interval(self.origin, start, end)
    }

    pub fn adjustments(&self) -> Result<Vec<ManualAdjustment>, String> {
        items(self.columns[11])
            .map(|text| self.adjustment(text))
            .collect()
    }

    /// Builds the input in the order the exporter does, so the first rejection is the same.
    pub fn calculate(&self) -> Result<AccountingResult, String> {
        let origin = self.origin;
        let policy =
            AccountingPolicy::new(self.lunch()?, duration(self.columns[6])).map_err(rejected)?;
        let adjustments = self.adjustments()?;
        let window = WorkdayWindow {
            workday: LocalWorkday::new(self.day, self.zone),
            start: origin,
            end: at(origin, self.window_end),
        };
        let (start, end) = self.columns[7].split_once(',').unwrap();
        let activity = items(self.columns[8]).map(|text| {
            let f: Vec<_> = text.split(',').collect();
            let state = match f[0] {
                "A" => ActivityState::Active,
                "F" => ActivityState::Afk,
                _ => ActivityState::Unknown,
            };
            ActivitySegment::new(interval(origin, f[1], f[2]).unwrap(), state, source(f[3]))
        });
        let coverage = ActivityCoverage::new(interval(origin, start, end).unwrap(), activity);
        let anchors: Vec<_> = self.columns[9].split(';').map(opt).collect();
        let events = WorkdayEventHistory::new(
            WorkdayStateAnchors {
                session: anchors[0].map(|text| session(self.recorded(text))),
                power: anchors[1].map(|text| power(self.recorded(text))),
                tracking: anchors[2].map(|text| tracking(self.recorded(text))),
            },
            items(self.columns[10]).map(|text| {
                let recorded = self.recorded(text);
                match &text[..1] {
                    "S" => session(recorded).into(),
                    "P" => power(recorded).into(),
                    _ => RecordedTransition::from(tracking(recorded)),
                }
            }),
        );
        let input = AccountingInput::new(
            window,
            at(origin, self.now),
            policy,
            coverage,
            events,
            adjustments,
        )
        .map_err(rejected)?;
        accounting::calculate(&input).map_err(rejected)
    }

    pub fn recorded(&self, text: &'a str) -> Recorded<&'a str> {
        let f: Vec<_> = text.split(',').collect();
        Recorded::new(
            WorkdayEventId(f[3].parse().unwrap()),
            at(self.origin, f[2]),
            f[1],
            source(f[4]),
        )
    }

    pub fn adjustment(&self, text: &str) -> Result<ManualAdjustment, String> {
        let f: Vec<_> = text.split(',').collect();
        let origin = self.origin;
        let interval = || interval(origin, f[6], f[7]);
        let kind = match f[0] {
            "I" => AdjustmentKind::IntervalWork {
                interval: interval()?,
            },
            "X" => AdjustmentKind::Exclusion {
                interval: interval()?,
            },
            "D" => AdjustmentKind::DurationWork {
                duration: duration(f[6]),
            },
            "R" => AdjustmentKind::Reclassification {
                interval: interval()?,
                classification: if f[8] == "C" {
                    ManualClassification::Credited
                } else {
                    ManualClassification::Excluded
                },
                confirms_review_id: opt(f[9]).map(|id| LongLockReviewId::new(id).unwrap()),
            },
            _ => AdjustmentKind::Revocation,
        };
        let adjustment = ManualAdjustment::new(
            AdjustmentId::new(f[1]).unwrap(),
            LocalWorkday::new(days_after(self.day, f[2].parse().unwrap()), self.zone),
            at(origin, f[3]),
            opt(f[4]).map(|id| AdjustmentId::new(id).unwrap()),
            kind,
        )
        .map_err(rejected)?;
        Ok(match opt(f[5]) {
            Some(reason) => adjustment.with_reason(&unescape(reason)),
            None => adjustment,
        })
    }
}

pub fn session(recorded: Recorded<&str>) -> Recorded<SessionState> {
    let state = match recorded.state {
        "Locked" => SessionState::Locked,
        "Unlocked" => SessionState::Unlocked,
        _ => SessionState::Unknown,
    };
    Recorded::new(recorded.id, recorded.occurred_at, state, recorded.origin)
}

pub fn power(recorded: Recorded<&str>) -> Recorded<PowerState> {
    let state = match recorded.state {
        "Active" => PowerState::Active,
        "Suspended" => PowerState::Suspended,
        _ => PowerState::Unknown,
    };
    Recorded::new(recorded.id, recorded.occurred_at, state, recorded.origin)
}

pub fn tracking(recorded: Recorded<&str>) -> Recorded<TrackingState> {
    let state = match recorded.state {
        "Running" => TrackingState::Running,
        "Paused" => TrackingState::Paused,
        _ => TrackingState::Finished,
    };
    Recorded::new(recorded.id, recorded.occurred_at, state, recorded.origin)
}

/// A case whose .NET expectation a later product decision changed: on line `line` of its
/// fixture, the expectation ends in `net`, and the Rust domain gives `now` there instead.
pub struct Revision {
    pub line: usize,
    pub net: &'static str,
    pub now: &'static str,
}

/// Checks every case of one fixture file and reports the differing lines together.
pub fn replay(name: &str, text: &str, check: impl Fn(&str) -> Result<String, String>) {
    replay_revised(name, text, &[], check);
}

/// [`replay`], expecting each of `revisions` instead of what .NET computed on its line.
pub fn replay_revised(
    name: &str,
    text: &str,
    revisions: &[Revision],
    check: impl Fn(&str) -> Result<String, String>,
) {
    let mut cases = 0;
    let mut differences = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.starts_with('#') {
            continue;
        }
        let (input, expected) = line
            .split_once(" => ")
            .unwrap_or_else(|| panic!("{name}.txt:{} has no expectation", index + 1));
        let expected = match revisions.iter().find(|revision| revision.line == index + 1) {
            None => expected.to_owned(),
            Some(revision) => {
                let kept = expected.strip_suffix(revision.net).unwrap_or_else(|| {
                    panic!("{name}.txt:{} does not end in {}", index + 1, revision.net)
                });
                format!("{kept}{}", revision.now)
            }
        };
        cases += 1;
        let actual = check(input).unwrap_or_else(|rejection| rejection);
        if actual != expected {
            differences.push(format!(
                "{name}.txt:{}: {input}\n  .NET: {expected}\n  Rust: {actual}",
                index + 1
            ));
        }
    }
    assert!(cases > 0, "{name}.txt has no cases");
    assert!(
        differences.is_empty(),
        "{} of {cases} {name} cases differ from .NET:\n{}",
        differences.len(),
        differences[..differences.len().min(10)].join("\n")
    );
}

/// The exception .NET throws where the Rust domain returns this error.
pub trait Rejection {
    fn exception(&self) -> String;
}

pub fn rejected(error: impl Rejection) -> String {
    format!("!{}", error.exception())
}

macro_rules! rejections {
    ($($error:ty { $($pattern:pat => $exception:expr),+ $(,)? })+) => {
        $(impl Rejection for $error {
            fn exception(&self) -> String {
                match self {
                    $($pattern => $exception.to_owned()),+
                }
            }
        })+
    };
}

rejections! {
    EmptyInterval { EmptyInterval => "ArgumentOutOfRangeException(end)" }
    NonPositiveReviewThreshold {
        NonPositiveReviewThreshold => "ArgumentOutOfRangeException(longLockReviewThreshold)",
    }
    InvalidAdjustment {
        InvalidAdjustment::NotAWorkingDay => "ArgumentException(workday)",
        InvalidAdjustment::NonPositiveDuration => "ArgumentOutOfRangeException(duration)",
        InvalidAdjustment::ConfirmationMustCredit => "ArgumentException(confirmsReviewId)",
        InvalidAdjustment::RevocationWithoutTarget => "unreachable: a .NET revocation always names its target",
    }
    InvalidAccountingInput {
        InvalidAccountingInput::AnchorNotBeforeWindow => "ArgumentException",
        InvalidAccountingInput::EmptyWindow | InvalidAccountingInput::TransitionOutsideWindow =>
            "ArgumentException(input)",
    }
    AdjustmentValidationError {
        error => {
            let name = format!("{error:?}");
            format!("Adjustment:{}", name.split([' ', '{']).next().unwrap())
        },
    }
    InvalidWeeklyCredit {
        InvalidWeeklyCredit::NominalTargetOutOfRange | InvalidWeeklyCredit::MinimumTargetOutOfRange =>
            "ArgumentOutOfRangeException(settings)",
        InvalidWeeklyCredit::WeekLength
        | InvalidWeeklyCredit::NotStartingOnMonday
        | InvalidWeeklyCredit::NotConsecutive => "ArgumentException(inputs)",
        InvalidWeeklyCredit::NegativeGrossWork | InvalidWeeklyCredit::AwaitingReviewOutOfRange =>
            "ArgumentOutOfRangeException(inputs)",
    }
    InvalidLedgerInput {
        InvalidLedgerInput::MissingRevision | InvalidLedgerInput::RevisionOutsideWeek =>
            "ArgumentException(durableRevisions)",
        InvalidLedgerInput::InvalidFridayWindow => "ArgumentException(fridayWindow)",
        InvalidLedgerInput::PreviousEntryOutsideWeek | InvalidLedgerInput::DuplicatePreviousEntry =>
            "ArgumentException(previousLedger)",
        InvalidLedgerInput::WeekLength
        | InvalidLedgerInput::NotStartingOnMonday
        | InvalidLedgerInput::NotConsecutive
        | InvalidLedgerInput::OpeningBalanceMismatch
        | InvalidLedgerInput::AppliedCreditUnavailable
        | InvalidLedgerInput::NegativeEarnedCredit
        | InvalidLedgerInput::ClosingBalanceMismatch => "ArgumentException(dailyResults)",
    }
    InvalidForecastInput {
        InvalidForecastInput::NegativeCreditedWork => "ArgumentOutOfRangeException(creditedWork)",
        InvalidForecastInput::NonPositiveTarget => "ArgumentOutOfRangeException(effectiveTarget)",
        InvalidForecastInput::NegativeWarningMargin =>
            "ArgumentOutOfRangeException(orangeWarningMargin)",
        InvalidForecastInput::MissingDepartureProjection =>
            "ArgumentNullException(projectedDeparture)",
    }
    InvalidDiagnostics {
        InvalidDiagnostics::OutsideRequestBounds | InvalidDiagnostics::Overlapping =>
            "ArgumentException(intervals)",
    }
    InvalidCalculation {
        InvalidCalculation::NonPositiveNominalTarget => "ArgumentOutOfRangeException(nominalTarget)",
        InvalidCalculation::NegativeWarningMargin =>
            "ArgumentOutOfRangeException(orangeWarningMargin)",
    }
    InvalidSummary {
        InvalidSummary::NonPositiveNominalTarget => "ArgumentOutOfRangeException(nominalTarget)",
        InvalidSummary::EffectiveTargetOutOfRange => "ArgumentOutOfRangeException(effectiveTarget)",
        InvalidSummary::CreditResultMismatch => "ArgumentException(creditResult)",
        InvalidSummary::IndicatorContradictsCompletion => "ArgumentException(indicatorState)",
        InvalidSummary::ProjectionContradictsCompletion => "ArgumentException(departureProjection)",
        InvalidSummary::CreditForAnotherDay
        | InvalidSummary::CreditForOtherWork
        | InvalidSummary::CreditForAnotherTarget => "ArgumentException(credit)",
    }
    CalendarError {
        CalendarError::EmptyInterval => "ArgumentOutOfRangeException(localEnd)",
        CalendarError::UnknownTimeZone(_) => "TimeZoneNotFoundException",
    }
}

pub const TICKS_PER_MINUTE: i64 = 600_000_000;

/// Reads `615`, `615+1` or `-5-3` as ticks.
pub fn ticks(token: &str) -> i64 {
    let split = token[1..].rfind(['+', '-']).map(|at| at + 1);
    let (minutes, rest) = split.map_or((token, "0"), |at| {
        let (minutes, rest) = token.split_at(at);
        (minutes, rest.strip_prefix('+').unwrap_or(rest))
    });
    minutes.parse::<i64>().unwrap() * TICKS_PER_MINUTE + rest.parse::<i64>().unwrap()
}

pub fn minutes(duration: SignedDuration) -> String {
    let ticks = i64::try_from(duration.as_nanos() / 100).unwrap();
    let minutes = (ticks + TICKS_PER_MINUTE / 2).div_euclid(TICKS_PER_MINUTE);
    match ticks - minutes * TICKS_PER_MINUTE {
        0 => minutes.to_string(),
        rest if rest > 0 => format!("{minutes}+{rest}"),
        rest => format!("{minutes}{rest}"),
    }
}

pub fn duration(token: &str) -> SignedDuration {
    SignedDuration::from_nanos(ticks(token) * 100)
}

pub fn at(origin: Timestamp, token: &str) -> Timestamp {
    origin + duration(token)
}

pub fn relative(instant: Timestamp, origin: Timestamp) -> String {
    minutes(instant.duration_since(origin))
}

pub fn interval(origin: Timestamp, start: &str, end: &str) -> Result<UtcInterval, String> {
    UtcInterval::new(at(origin, start), at(origin, end)).map_err(rejected)
}

pub fn time_of_day(token: &str) -> Time {
    Time::midnight().checked_add(duration(token)).unwrap()
}

pub fn date(text: &str) -> Date {
    text.parse().unwrap()
}

pub fn days_after(date: Date, days: i64) -> Date {
    date.checked_add(Span::new().days(days)).unwrap()
}

pub fn midnight(date: Date) -> Timestamp {
    date.to_zoned(TimeZone::UTC).unwrap().timestamp()
}

pub fn source(id: &str) -> AutomaticOrigin {
    AutomaticOrigin::new(id).unwrap()
}

pub fn flag(token: &str) -> bool {
    token == "1"
}

pub fn bit(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

pub fn opt(token: &str) -> Option<&str> {
    (token != "_").then_some(token)
}

pub fn items(column: &str) -> impl Iterator<Item = &str> {
    column.split(';').filter(|item| !item.is_empty())
}

/// Undoes the exporter's `\u{hex}` escapes.
pub fn unescape(text: &str) -> String {
    let mut unescaped = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("\\u{") {
        let end = start + rest[start..].find('}').unwrap();
        unescaped.push_str(&rest[..start]);
        let code = u32::from_str_radix(&rest[start + 3..end], 16).unwrap();
        unescaped.push(char::from_u32(code).unwrap());
        rest = &rest[end + 1..];
    }
    unescaped.push_str(rest);
    unescaped
}
