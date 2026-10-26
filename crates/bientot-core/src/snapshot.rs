//! The daily summary a weekly recalculation stores, ported from
//! `Services/DailySummarySnapshotFactory.cs`.
//!
//! Each stored revision carries a fingerprint of the material it was calculated from. A
//! recalculation whose fingerprint equals the stored head's reuses that revision instead of
//! appending one, so the fingerprint is written byte for byte as .NET writes it: the same JSON,
//! key order and escaping, hashed with SHA-256. Otherwise the first Rust run would append a new
//! revision for every day the .NET app recorded.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::Write as _;

use bientot_domain::accounting::{
    AccountedOrigin, AccountingLabel, AccountingResult, LongLockReview, LongLockReviewReason,
    SegmentKind,
};
use bientot_domain::configuration::Settings;
use bientot_domain::credit::DailyCreditResult;
use bientot_domain::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
    DailySummarySnapshot, DailySummarySnapshotParts,
};
use bientot_domain::sources::{SourceDiagnostics, SourceIntervalKind, SourceQuality};
use bientot_domain::summaries::{DailyAccountingCalculation, DailySummary, IndicatorState};
use bientot_store::codec::{format_date, write_json_string};
use jiff::civil::Time;
use jiff::{SignedDuration, Timestamp};
use sha2::{Digest, Sha256};

const FINGERPRINT_PREFIX: &str = "daily-summary-material:v1:sha256:";
/// .NET ticks between 0001-01-01 and the Unix epoch.
const UNIX_EPOCH_TICKS: i128 = 621_355_968_000_000_000;

/// The stored summary of a weekday. Once Friday has ended, `expire_friday_credit` turns its
/// closing credit from available into expired. `debt_pending` marks a day still running while
/// debt is on, whose shortfall is not in the debt yet.
///
/// # Panics
///
/// When the calculation, credit and summary belong to different days, or the day is a weekend,
/// which history does not hold.
pub fn create(
    calculation: &DailyAccountingCalculation,
    credit: DailyCreditResult,
    summary: &DailySummary,
    settings: &Settings,
    expire_friday_credit: bool,
    debt_pending: bool,
) -> DailySummarySnapshot {
    assert!(
        calculation.workday() == summary.workday() && credit.date() == calculation.workday().date,
        "The calculation, credit result, and summary must belong to the same workday."
    );
    let (carry_available, carry_expired) = if expire_friday_credit {
        (SignedDuration::ZERO, credit.closing_balance())
    } else {
        (credit.closing_balance(), SignedDuration::ZERO)
    };
    let accounting = calculation.accounting();
    let timeline = accounting.timeline();
    let parts = DailySummarySnapshotParts {
        workday: calculation.workday().clone(),
        computed_at: summary.computed_at(),
        input_fingerprint: String::new(),
        credited_work: accounting.credited_work(),
        excluded_work: timeline.excluded_work(),
        unknown_work: timeline.unknown_work(),
        nominal_target: credit.nominal_target(),
        effective_target: credit.effective_target(),
        carry_available,
        carry_applied: credit.applied_credit(),
        carry_earned: credit.earned_transferable_credit(),
        carry_expired,
        carry_debt: credit.closing_debt(),
        debt_pending,
        projected_departure: summary
            .departure_projection()
            .map(|projection| projection.projected_departure),
        indicator_state: summary.indicator_state(),
        source_quality: calculation.source_diagnostics().operation().quality,
        has_pending_review: !accounting.pending_reviews().is_empty(),
        breakdown: breakdown(accounting),
    };
    let input_fingerprint = fingerprint(calculation, &credit, settings, &parts);
    DailySummarySnapshot::new(DailySummarySnapshotParts {
        input_fingerprint,
        ..parts
    })
    .expect("a summary of a consistent weekday calculation is a valid snapshot")
}

fn breakdown(accounting: &AccountingResult) -> DailySummaryBreakdown {
    // Only locks still awaiting a decision carry their review id, so history offers to resolve
    // exactly those.
    let pending: HashSet<_> = accounting
        .pending_reviews()
        .iter()
        .map(|review| &review.id)
        .collect();
    let timed = accounting.timeline().segments().iter().map(|segment| {
        let (disposition, review_id) = match segment.kind() {
            SegmentKind::Credited => (DailySummaryDisposition::ConfirmedCredit, None),
            SegmentKind::ProvisionalCredit { review_id } => (
                DailySummaryDisposition::ProvisionalCredit,
                pending.contains(review_id).then(|| review_id.clone()),
            ),
            SegmentKind::Excluded => (DailySummaryDisposition::Excluded, None),
            SegmentKind::Unknown => (DailySummaryDisposition::Unknown, None),
        };
        DailySummaryBreakdownEntry::new(
            disposition,
            segment.label(),
            segment.interval().duration(),
            Some(segment.interval()),
            segment.origin().manual_adjustment_id().cloned(),
            review_id,
        )
        .expect("accounted segments are not empty")
    });
    let manual = accounting.manual_duration_credits().iter().map(|credit| {
        DailySummaryBreakdownEntry::new(
            DailySummaryDisposition::ManualCredit,
            AccountingLabel::Manual,
            credit.duration(),
            None,
            Some(credit.adjustment_id().clone()),
            None,
        )
        .expect("manual duration credits are positive")
    });
    DailySummaryBreakdown::new(timed.chain(manual))
}

fn fingerprint(
    calculation: &DailyAccountingCalculation,
    credit: &DailyCreditResult,
    settings: &Settings,
    durable: &DailySummarySnapshotParts,
) -> String {
    let accounting = calculation.accounting();
    let timeline = accounting.timeline();
    let mut writer = JsonWriter::default();
    writer.start_object();
    writer.string("workday", &format_date(calculation.workday().date));
    writer.string("timeZoneId", &calculation.workday().time_zone_id);
    writer.instant("windowStartUtcTicks", timeline.window().start);
    writer.instant("windowEndUtcTicks", timeline.window().end);
    writer.instant("throughUtcTicks", timeline.through());
    writer.instant("resolvedLunchStartUtcTicks", calculation.lunch().start());
    writer.instant("resolvedLunchEndUtcTicks", calculation.lunch().end());
    writer.instant(
        "resolvedPreferredDepartureUtcTicks",
        calculation.preferred_latest_departure(),
    );
    write_timeline(&mut writer, accounting);
    write_reviews(&mut writer, accounting.pending_reviews());
    write_source(&mut writer, calculation.source_diagnostics());
    write_settings(&mut writer, settings);
    write_weekly_result(&mut writer, credit, durable);
    writer.optional_instant("projectedDepartureUtcTicks", durable.projected_departure);
    writer.string("indicatorState", indicator_name(durable.indicator_state));
    write_durable_fields(&mut writer, durable);
    writer.end_object();

    let mut fingerprint = FINGERPRINT_PREFIX.to_owned();
    for byte in Sha256::digest(writer.json.as_bytes()) {
        write!(fingerprint, "{byte:02x}").expect("writing to a String cannot fail");
    }
    fingerprint
}

fn write_timeline(writer: &mut JsonWriter, accounting: &AccountingResult) {
    let mut segments: Vec<_> = accounting.timeline().segments().iter().collect();
    segments.sort_by_key(|segment| (segment.interval().start(), segment.interval().end()));
    writer.start_array("timeline");
    for segment in segments {
        writer.start_object();
        let (disposition, review_id) = match segment.kind() {
            SegmentKind::Credited => ("ConfirmedCredit", None),
            SegmentKind::ProvisionalCredit { review_id } => {
                ("ProvisionalCredit", Some(review_id.as_str()))
            }
            SegmentKind::Excluded => ("Excluded", None),
            SegmentKind::Unknown => ("Unknown", None),
        };
        writer.string("disposition", disposition);
        writer.string("label", label_name(segment.label()));
        writer.instant("startUtcTicks", segment.interval().start());
        writer.instant("endUtcTicks", segment.interval().end());
        writer.optional_string("reviewId", review_id);
        writer.key("origin");
        write_origin(writer, segment.origin());
        writer.end_object();
    }
    writer.end_array();

    let mut credits: Vec<_> = accounting.manual_duration_credits().iter().collect();
    credits.sort_by(|left, right| {
        ordinal(
            left.adjustment_id().as_str(),
            right.adjustment_id().as_str(),
        )
    });
    writer.start_array("manualCredits");
    for credit in credits {
        writer.start_object();
        writer.string("adjustmentId", credit.adjustment_id().as_str());
        writer.duration("durationTicks", credit.duration());
        writer.end_object();
    }
    writer.end_array();
}

fn write_origin(writer: &mut JsonWriter, origin: &AccountedOrigin) {
    writer.start_object();
    match origin {
        AccountedOrigin::Automatic(automatic) => {
            writer.string("variant", "Automatic");
            writer.string("sourceId", automatic.source_id());
        }
        AccountedOrigin::Manual {
            adjustment_id,
            replaces,
        } => {
            writer.string("variant", "Manual");
            writer.string("adjustmentId", adjustment_id.as_str());
            writer.key("replaces");
            write_origin(writer, replaces);
        }
    }
    writer.end_object();
}

fn write_reviews(writer: &mut JsonWriter, reviews: &[LongLockReview]) {
    let mut reviews: Vec<_> = reviews.iter().collect();
    reviews.sort_by(|left, right| {
        ordinal(left.id.as_str(), right.id.as_str())
            .then(
                left.locked_episode
                    .start()
                    .cmp(&right.locked_episode.start()),
            )
            .then(left.locked_episode.end().cmp(&right.locked_episode.end()))
            .then(ordinal(reason_name(left.reason), reason_name(right.reason)))
            .then(left.is_open.cmp(&right.is_open))
            .then(coverage_key(left).cmp(&coverage_key(right)))
    });
    writer.start_array("pendingReviews");
    for review in reviews {
        writer.start_object();
        writer.string("id", review.id.as_str());
        writer.instant("lockedStartUtcTicks", review.locked_episode.start());
        writer.instant("lockedEndUtcTicks", review.locked_episode.end());
        writer.string("reason", reason_name(review.reason));
        writer.boolean("isOpen", review.is_open);
        writer.start_array("provisionalCoverage");
        for interval in review.provisional_coverage.iter() {
            writer.start_object();
            writer.instant("startUtcTicks", interval.start());
            writer.instant("endUtcTicks", interval.end());
            writer.end_object();
        }
        writer.end_array();
        writer.end_object();
    }
    writer.end_array();
}

/// The provisional coverage as .NET orders reviews by it: `start:end` ticks joined by `;`.
fn coverage_key(review: &LongLockReview) -> String {
    review
        .provisional_coverage
        .iter()
        .map(|interval| format!("{}:{}", ticks(interval.start()), ticks(interval.end())))
        .collect::<Vec<_>>()
        .join(";")
}

fn write_source(writer: &mut JsonWriter, diagnostics: &SourceDiagnostics) {
    writer.start_object_property("source");
    writer.string("sourceId", &diagnostics.operation().source_id);
    writer.instant("requestStartUtcTicks", diagnostics.request_bounds().start());
    writer.instant("requestEndUtcTicks", diagnostics.request_bounds().end());
    writer.string("quality", quality_name(diagnostics.operation().quality));
    let mut intervals: Vec<_> = diagnostics.intervals().iter().collect();
    intervals.sort_by_key(|interval| (interval.interval().start(), interval.interval().end()));
    writer.start_array("diagnostics");
    for interval in intervals {
        writer.start_object();
        writer.instant("startUtcTicks", interval.interval().start());
        writer.instant("endUtcTicks", interval.interval().end());
        writer.string("kind", interval_kind_name(interval.kind()));
        writer.string("sourceId", interval.source_id());
        writer.end_object();
    }
    writer.end_array();
    writer.end_object();
}

fn write_settings(writer: &mut JsonWriter, settings: &Settings) {
    let settlement = &settings.activity_watch.settlement;
    writer.start_object_property("settings");
    writer.duration("nominalTargetTicks", settings.nominal_daily_target);
    writer.duration(
        "minimumEffectiveTargetTicks",
        settings.minimum_effective_target,
    );
    writer.boolean("weeklyCreditEnabled", settings.weekly_credit_enabled);
    writer.time_of_day("lunchStartTicks", settings.lunch_start);
    writer.time_of_day("lunchEndTicks", settings.lunch_end);
    writer.duration(
        "longLockThresholdTicks",
        settings.long_lock_review_threshold,
    );
    writer.time_of_day(
        "preferredDepartureTicks",
        settings.preferred_latest_departure,
    );
    writer.duration("orangeMarginTicks", settings.orange_warning_margin);
    writer.optional_string(
        "selectedBucketId",
        settings.activity_watch.selected_bucket_id.as_deref(),
    );
    writer.duration("settlementAfkTimeoutTicks", settlement.afk_timeout);
    writer.duration("settlementPollIntervalTicks", settlement.poll_interval);
    writer.duration("settlementMaximumJitterTicks", settlement.maximum_jitter);
    // Debt fields are written only when debt is on, so fingerprints from before debt existed
    // stay stable.
    if settings.weekly_debt_enabled {
        writer.boolean("weeklyDebtEnabled", true);
        match settings.maximum_weekly_debt {
            Some(maximum) => writer.duration("maximumWeeklyDebtTicks", maximum),
            None => writer.null("maximumWeeklyDebtTicks"),
        }
    }
    writer.end_object();
}

fn write_weekly_result(
    writer: &mut JsonWriter,
    credit: &DailyCreditResult,
    durable: &DailySummarySnapshotParts,
) {
    writer.start_object_property("weeklyCredit");
    writer.duration("openingBalanceTicks", credit.opening_balance());
    writer.duration("appliedTicks", credit.applied_credit());
    writer.duration("earnedTicks", credit.earned_transferable_credit());
    writer.duration("closingBalanceTicks", credit.closing_balance());
    writer.duration("availableTicks", durable.carry_available);
    writer.duration("expiredTicks", durable.carry_expired);
    if !credit.opening_debt().is_zero() || !credit.closing_debt().is_zero() {
        writer.duration("openingDebtTicks", credit.opening_debt());
        writer.duration("repaidDebtTicks", credit.repaid_debt());
        writer.duration("incurredDebtTicks", credit.incurred_debt());
        writer.duration("closingDebtTicks", credit.closing_debt());
    }
    writer.end_object();
}

fn write_durable_fields(writer: &mut JsonWriter, durable: &DailySummarySnapshotParts) {
    writer.start_object_property("durableSnapshot");
    writer.duration("creditedWorkTicks", durable.credited_work);
    writer.duration("excludedWorkTicks", durable.excluded_work);
    writer.duration("unknownWorkTicks", durable.unknown_work);
    writer.duration("nominalTargetTicks", durable.nominal_target);
    writer.duration("effectiveTargetTicks", durable.effective_target);
    writer.duration("carryAvailableTicks", durable.carry_available);
    writer.duration("carryAppliedTicks", durable.carry_applied);
    writer.duration("carryEarnedTicks", durable.carry_earned);
    writer.duration("carryExpiredTicks", durable.carry_expired);
    if !durable.carry_debt.is_zero() {
        writer.duration("carryDebtTicks", durable.carry_debt);
    }
    writer.optional_instant("projectedDepartureUtcTicks", durable.projected_departure);
    writer.string("indicatorState", indicator_name(durable.indicator_state));
    writer.string("sourceQuality", quality_name(durable.source_quality));
    writer.boolean("hasPendingReview", durable.has_pending_review);
    writer.start_array("breakdown");
    for entry in durable.breakdown.entries() {
        writer.start_object();
        writer.string("disposition", disposition_name(entry.disposition()));
        writer.string("label", label_name(entry.label()));
        writer.duration("durationTicks", entry.duration());
        writer.optional_instant(
            "intervalStartUtcTicks",
            entry.interval().map(|interval| interval.start()),
        );
        writer.optional_instant(
            "intervalEndUtcTicks",
            entry.interval().map(|interval| interval.end()),
        );
        writer.optional_string("adjustmentId", entry.adjustment_id().map(|id| id.as_str()));
        writer.optional_string("reviewId", entry.review_id().map(|id| id.as_str()));
        writer.end_object();
    }
    writer.end_array();
    writer.end_object();
}

/// `StringComparer.Ordinal`: UTF-16 code units, which order some characters beyond the BMP
/// differently from UTF-8 bytes.
fn ordinal(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

/// `DateTimeOffset.UtcTicks`; .NET keeps 100 ns ticks, so a finer instant is truncated.
fn ticks(instant: Timestamp) -> i128 {
    instant.as_nanosecond().div_euclid(100) + UNIX_EPOCH_TICKS
}

/// The compact JSON of a .NET `Utf8JsonWriter` with its default options, for the members this
/// module writes.
#[derive(Default)]
struct JsonWriter {
    json: String,
    /// A value or a closed container precedes, so the next member or element needs a comma.
    needs_comma: bool,
}

impl JsonWriter {
    fn separate(&mut self) {
        if self.needs_comma {
            self.json.push(',');
        }
    }

    fn key(&mut self, name: &str) {
        self.separate();
        write_json_string(&mut self.json, name);
        self.json.push(':');
        self.needs_comma = false;
    }

    fn raw_value(&mut self, value: impl std::fmt::Display) {
        self.separate();
        write!(self.json, "{value}").expect("writing to a String cannot fail");
        self.needs_comma = true;
    }

    fn start_object(&mut self) {
        self.separate();
        self.json.push('{');
        self.needs_comma = false;
    }

    fn start_object_property(&mut self, name: &str) {
        self.key(name);
        self.start_object();
    }

    fn end_object(&mut self) {
        self.json.push('}');
        self.needs_comma = true;
    }

    fn start_array(&mut self, name: &str) {
        self.key(name);
        self.json.push('[');
        self.needs_comma = false;
    }

    fn end_array(&mut self) {
        self.json.push(']');
        self.needs_comma = true;
    }

    fn string(&mut self, name: &str, value: &str) {
        self.key(name);
        write_json_string(&mut self.json, value);
        self.needs_comma = true;
    }

    fn optional_string(&mut self, name: &str, value: Option<&str>) {
        match value {
            Some(value) => self.string(name, value),
            None => self.null(name),
        }
    }

    fn null(&mut self, name: &str) {
        self.key(name);
        self.raw_value("null");
    }

    fn boolean(&mut self, name: &str, value: bool) {
        self.key(name);
        self.raw_value(value);
    }

    fn instant(&mut self, name: &str, instant: Timestamp) {
        self.key(name);
        self.raw_value(ticks(instant));
    }

    fn optional_instant(&mut self, name: &str, instant: Option<Timestamp>) {
        match instant {
            Some(instant) => self.instant(name, instant),
            None => self.null(name),
        }
    }

    /// `TimeSpan.Ticks`, truncated toward zero.
    fn duration(&mut self, name: &str, duration: SignedDuration) {
        self.key(name);
        self.raw_value(duration.as_nanos() / 100);
    }

    /// `TimeOnly.Ticks`: the ticks since midnight.
    fn time_of_day(&mut self, name: &str, time: Time) {
        self.duration(name, time.duration_since(Time::midnight()));
    }
}

fn label_name(label: AccountingLabel) -> &'static str {
    match label {
        AccountingLabel::ActiveWork => "ActiveWork",
        AccountingLabel::Afk => "Afk",
        AccountingLabel::LunchLock => "LunchLock",
        AccountingLabel::LockedWork => "LockedWork",
        AccountingLabel::Suspended => "Suspended",
        AccountingLabel::ManualPause => "ManualPause",
        AccountingLabel::DayFinished => "DayFinished",
        AccountingLabel::Manual => "Manual",
        AccountingLabel::Unknown => "Unknown",
        AccountingLabel::OvernightLock => "OvernightLock",
        AccountingLabel::ScheduledAway => "ScheduledAway",
        AccountingLabel::RebuiltWork => "RebuiltWork",
    }
}

fn disposition_name(disposition: DailySummaryDisposition) -> &'static str {
    match disposition {
        DailySummaryDisposition::ConfirmedCredit => "ConfirmedCredit",
        DailySummaryDisposition::ProvisionalCredit => "ProvisionalCredit",
        DailySummaryDisposition::Excluded => "Excluded",
        DailySummaryDisposition::Unknown => "Unknown",
        DailySummaryDisposition::ManualCredit => "ManualCredit",
    }
}

fn reason_name(reason: LongLockReviewReason) -> &'static str {
    match reason {
        LongLockReviewReason::ThresholdExceeded => "ThresholdExceeded",
        LongLockReviewReason::RecurringAppointment => "RecurringAppointment",
        LongLockReviewReason::RebuiltSession => "RebuiltSession",
    }
}

fn quality_name(quality: SourceQuality) -> &'static str {
    match quality {
        SourceQuality::Complete => "Complete",
        SourceQuality::Incomplete => "Incomplete",
        SourceQuality::Ambiguous => "Ambiguous",
        SourceQuality::Unauthorized => "Unauthorized",
        SourceQuality::Incompatible => "Incompatible",
        SourceQuality::Malformed => "Malformed",
    }
}

fn interval_kind_name(kind: SourceIntervalKind) -> &'static str {
    match kind {
        SourceIntervalKind::PendingSettlement => "PendingSettlement",
        SourceIntervalKind::InteriorCompatibilitySeam => "InteriorCompatibilitySeam",
        SourceIntervalKind::UnresolvedCoverage => "UnresolvedCoverage",
    }
}

fn indicator_name(state: IndicatorState) -> &'static str {
    match state {
        IndicatorState::Green => "Green",
        IndicatorState::Orange => "Orange",
        IndicatorState::Red => "Red",
        IndicatorState::Blue => "Blue",
    }
}

#[cfg(test)]
mod tests;
