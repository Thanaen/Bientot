//! The weekly recalculation that turns workday calculations into stored summaries and credit,
//! ported from `Services/WeeklyRecalculationService.cs`, and the state a refresh publishes,
//! ported from `Services/WorkdayRefreshState.cs`.
//!
//! Every refresh first closes the earlier weeks still waiting for their Friday expiry, then
//! recalculates the current week up to today and commits its summaries and ledger at once. A day
//! whose fingerprint matches its stored head keeps that revision, so repeated refreshes append
//! nothing.

use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::Timestamp;
use jiff::civil::{Date, Weekday};
use worktime_domain::credit::{
    CreditLedgerEntry, DailyCreditInput, InvalidLedgerInput, build_ledger, calculate_week,
};
use worktime_domain::history::{DailySummaryRevision, DailySummarySnapshot};
use worktime_domain::sources::SourceDiagnostics;
use worktime_domain::summaries::{self, DailySummary};
use worktime_domain::time::{LocalWorkday, UnknownTimeZone};
use worktime_store::codec::format_date;
use worktime_store::credit::{
    WeeklyCreditError, WeeklyCreditRepository, WeeklyCreditState, WeeklyRevisionVector,
};
use worktime_store::history;

use crate::accounting::{
    AccountingError, ActivityReadDisposition, CalculationContext, WorkdayAccounting,
    WorkdayCalculation,
};
use crate::snapshot;

/// A commit that finds the week changed since it was read starts over from a fresh read, at
/// most this many times in all.
const MAX_COMMIT_ATTEMPTS: u32 = 3;

/// Where the weekly recalculation reads and commits weeks: the ledger, or a test double that
/// counts or contends.
pub trait WeeklyCreditStore {
    fn read_pending_expiry_weeks(
        &self,
        before_monday: Date,
    ) -> Result<Vec<Date>, WeeklyCreditError>;

    fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError>;

    fn commit(
        &self,
        monday: Date,
        expected_revisions: &WeeklyRevisionVector,
        revisions_to_append: &[DailySummaryRevision],
        replacement_ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError>;
}

impl WeeklyCreditStore for WeeklyCreditRepository<'_> {
    fn read_pending_expiry_weeks(
        &self,
        before_monday: Date,
    ) -> Result<Vec<Date>, WeeklyCreditError> {
        WeeklyCreditRepository::read_pending_expiry_weeks(self, before_monday)
    }

    fn read_week(&self, monday: Date) -> Result<WeeklyCreditState, WeeklyCreditError> {
        WeeklyCreditRepository::read_week(self, monday)
    }

    fn commit(
        &self,
        monday: Date,
        expected_revisions: &WeeklyRevisionVector,
        revisions_to_append: &[DailySummaryRevision],
        replacement_ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        WeeklyCreditRepository::commit(
            self,
            monday,
            expected_revisions,
            revisions_to_append,
            replacement_ledger,
        )
    }
}

/// A day whose stored summary could only be replaced from evidence the source did not give.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEvidenceUnavailable {
    pub workday_date: Date,
    pub diagnostics: SourceDiagnostics,
    pub disposition: ActivityReadDisposition,
}

impl std::fmt::Display for SourceEvidenceUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let disposition = match self.disposition {
            ActivityReadDisposition::Snapshot => "Snapshot",
            ActivityReadDisposition::Unavailable => "Unavailable",
            ActivityReadDisposition::NotRequested => "NotRequested",
        };
        write!(
            f,
            "The source has no usable evidence for workday {} ({disposition}).",
            format_date(self.workday_date)
        )
    }
}

#[derive(Debug)]
pub enum WeeklyError {
    /// Nothing was written; the week keeps its stored summaries until the source answers.
    SourceEvidenceUnavailable(Box<SourceEvidenceUnavailable>),
    Accounting(AccountingError),
    /// Includes [`WeeklyCreditError::Stale`] once every attempt found the week changed.
    Store(WeeklyCreditError),
    /// A stored week names a zone this machine does not know.
    UnknownTimeZone(UnknownTimeZone),
    /// The clock is before the stored head of this later day.
    ClockRollbackDropsWorkday(Date),
    /// The clock is before the instant the stored head of this day was computed.
    ClockRollbackPrecedesMaterial(Date),
    /// A recalculation has the stored head's fingerprint but other durable values.
    FingerprintCollision(Date),
    /// The stored heads of the past week starting this Monday name several zones.
    MixedTimeZones(Date),
    /// The stored ledger of the week cannot be extended.
    Ledger(InvalidLedgerInput),
}

impl std::fmt::Display for WeeklyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceEvidenceUnavailable(error) => error.fmt(f),
            Self::Accounting(error) => error.fmt(f),
            Self::Store(error) => error.fmt(f),
            Self::UnknownTimeZone(error) => error.fmt(f),
            Self::ClockRollbackDropsWorkday(date) => write!(
                f,
                "Clock rollback would drop persisted workday {}.",
                format_date(*date)
            ),
            Self::ClockRollbackPrecedesMaterial(date) => write!(
                f,
                "Clock rollback precedes persisted material for {}.",
                format_date(*date)
            ),
            Self::FingerprintCollision(date) => write!(
                f,
                "Equal material fingerprints disagree for {}.",
                format_date(*date)
            ),
            Self::MixedTimeZones(monday) => write!(
                f,
                "Historical week {} must have one persisted timezone.",
                format_date(*monday)
            ),
            Self::Ledger(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for WeeklyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Accounting(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::UnknownTimeZone(error) => Some(error),
            Self::Ledger(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AccountingError> for WeeklyError {
    fn from(error: AccountingError) -> Self {
        Self::Accounting(error)
    }
}

impl From<WeeklyCreditError> for WeeklyError {
    fn from(error: WeeklyCreditError) -> Self {
        Self::Store(error)
    }
}

/// What a refresh publishes: the current summary, or the last one beside the source failure that
/// kept it from being replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkdayRefreshState {
    pub summary: Option<DailySummary>,
    pub source_failure: Option<SourceEvidenceUnavailable>,
}

/// The days a recalculation covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeekPlan {
    /// The current week through `today`, whose summary is returned; a weekend covers the whole
    /// workweek.
    CurrentObservedPrefix { monday: Date, today: Date },
    /// A past week, replayed whole in the zone it was stored in to commit its Friday expiry.
    HistoricalClosure { monday: Date },
}

impl WeekPlan {
    fn monday(self) -> Date {
        match self {
            Self::CurrentObservedPrefix { monday, .. } | Self::HistoricalClosure { monday } => {
                monday
            }
        }
    }

    fn workday_dates(self) -> Vec<Date> {
        let count = match self {
            Self::CurrentObservedPrefix { monday, today } if !is_weekend(today) => {
                (today - monday).get_days() + 1
            }
            _ => 5,
        };
        (0..count)
            .map(|offset| days_after(self.monday(), offset.into()))
            .collect()
    }

    fn summary_date(self) -> Option<Date> {
        match self {
            Self::CurrentObservedPrefix { today, .. } => Some(today),
            Self::HistoricalClosure { .. } => None,
        }
    }
}

/// Recalculates weeks through `accounting` and commits them to `store`. Calls are expected one at
/// a time; a concurrent writer is detected by the store's revision check and retried.
pub struct WeeklyRecalculation<'a> {
    accounting: &'a WorkdayAccounting<'a>,
    store: &'a dyn WeeklyCreditStore,
}

impl<'a> WeeklyRecalculation<'a> {
    pub fn new(accounting: &'a WorkdayAccounting<'a>, store: &'a dyn WeeklyCreditStore) -> Self {
        Self { accounting, store }
    }

    /// Closes the past weeks still owed their expiry, then commits the current week and returns
    /// today's summary. When the clock crosses into another week meanwhile, that week's
    /// predecessors are closed first.
    pub fn calculate_current(&self) -> Result<DailySummary, WeeklyError> {
        let context = self.accounting.capture_context();
        let mut maintained_monday =
            start_of_week(context.calendar().workday(context.observed_at).date);
        self.close_pending_weeks(maintained_monday)?;

        let mut attempt = 1;
        loop {
            let context = self.accounting.capture_context();
            let today = context.calendar().workday(context.observed_at).date;
            let monday = start_of_week(today);
            if monday != maintained_monday {
                self.close_pending_weeks(monday)?;
                maintained_monday = monday;
                attempt = 1;
                continue;
            }
            match self.recalculate_week(WeekPlan::CurrentObservedPrefix { monday, today }, context)
            {
                Ok(summary) => {
                    return Ok(summary.expect("the current plan includes today"));
                }
                Err(WeeklyError::Store(WeeklyCreditError::Stale { .. }))
                    if attempt < MAX_COMMIT_ATTEMPTS =>
                {
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// [`Self::calculate_current`] as a refresh publishes it: a source failure keeps the summary
    /// of `previous` instead of failing the refresh.
    pub fn refresh(
        &self,
        previous: Option<&WorkdayRefreshState>,
    ) -> Result<WorkdayRefreshState, WeeklyError> {
        match self.calculate_current() {
            Ok(summary) => Ok(WorkdayRefreshState {
                summary: Some(summary),
                source_failure: None,
            }),
            Err(WeeklyError::SourceEvidenceUnavailable(failure)) => Ok(WorkdayRefreshState {
                summary: previous.and_then(|state| state.summary.clone()),
                source_failure: Some(*failure),
            }),
            Err(error) => Err(error),
        }
    }

    /// A week the source cannot replay stays pending for a later refresh and does not hold up
    /// the others.
    fn close_pending_weeks(&self, before_monday: Date) -> Result<(), WeeklyError> {
        for monday in self.store.read_pending_expiry_weeks(before_monday)? {
            match self.close_historical_week(monday) {
                Ok(()) | Err(WeeklyError::SourceEvidenceUnavailable(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn close_historical_week(&self, monday: Date) -> Result<(), WeeklyError> {
        let mut attempt = 1;
        loop {
            let context = self.accounting.capture_context();
            match self.recalculate_week(WeekPlan::HistoricalClosure { monday }, context) {
                Ok(_) => return Ok(()),
                Err(WeeklyError::Store(WeeklyCreditError::Stale { .. }))
                    if attempt < MAX_COMMIT_ATTEMPTS =>
                {
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn recalculate_week(
        &self,
        plan: WeekPlan,
        captured: CalculationContext,
    ) -> Result<Option<DailySummary>, WeeklyError> {
        let monday = plan.monday();
        let state = self.store.read_week(monday)?;
        let context = self.resolve_context(plan, &state, captured)?;
        let workday_dates = plan.workday_dates();
        validate_clock_has_not_moved_backward(&state, &workday_dates, context.observed_at)?;

        let persisted: HashMap<Date, &DailySummaryRevision> = state
            .latest_revisions()
            .iter()
            .map(|revision| (revision.workday_date(), revision))
            .collect();
        let historical = matches!(plan, WeekPlan::HistoricalClosure { .. });
        let mut calculations = Vec::with_capacity(workday_dates.len());
        for &date in &workday_dates {
            let calculation = self.accounting.calculate(date, &context)?;
            // A stored day is only replaced from a usable read, and a past week only from a
            // complete one that knew something about the day. A day before the first selection
            // has no source to wait for.
            let evidence_required = (historical || persisted.contains_key(&date))
                && !calculation.predates_first_selection();
            let evidence_available = if historical {
                calculation.has_historical_replay_evidence()
            } else {
                calculation.has_usable_source_snapshot()
            };
            if evidence_required && !evidence_available {
                return Err(WeeklyError::SourceEvidenceUnavailable(Box::new(
                    SourceEvidenceUnavailable {
                        workday_date: date,
                        diagnostics: calculation.summary_input.source_diagnostics().clone(),
                        disposition: calculation.activity_read,
                    },
                )));
            }
            calculations.push(calculation);
        }

        // A day can leave debt only once its window has ended; the running day never does.
        let finished: Vec<bool> = calculations
            .iter()
            .map(|calculation| context.observed_at >= window_end(calculation))
            .collect();
        let credit_inputs: Vec<_> = calculations
            .iter()
            .zip(&finished)
            .map(|(calculation, &is_finished)| {
                let accounting = calculation.summary_input.accounting();
                DailyCreditInput {
                    date: calculation.summary_input.workday().date,
                    gross_credited_work: accounting.credited_work(),
                    work_awaiting_review: accounting.work_awaiting_review(),
                    is_finished,
                }
            })
            .collect();
        let credit_results = calculate_week(&credit_inputs, &context.settings)
            .expect("validated settings and a consecutive week from Monday");
        // A running day's shortfall still has to reach the debt, so its week stays on the
        // closure worklist.
        let debt_enabled =
            context.settings.weekly_credit_enabled && context.settings.weekly_debt_enabled;
        let calendar = context.calendar();
        let friday = days_after(monday, 4);
        let friday_window = calendar
            .window(&LocalWorkday::new(friday, context.time_zone_id.clone()))
            .map_err(AccountingError::from)?;

        let revision_vector = state.revision_vector();
        let mut selected_revisions = BTreeMap::new();
        let mut revisions_to_append = Vec::new();
        let mut current_summary = None;
        for ((calculation, &credit), &is_finished) in
            calculations.iter().zip(&credit_results).zip(&finished)
        {
            let date = calculation.summary_input.workday().date;
            let summary = summaries::build_with_credit(&calculation.summary_input, credit)
                .expect("the credit result was calculated for this day");
            let expire_friday_credit = date == friday && context.observed_at >= friday_window.end;
            let candidate = snapshot::create(
                &calculation.summary_input,
                credit,
                &summary,
                &context.settings,
                expire_friday_credit,
                debt_enabled && !is_finished,
            );
            let selected = match persisted.get(&date) {
                Some(head)
                    if head.snapshot().input_fingerprint() == candidate.input_fingerprint() =>
                {
                    // The head was read back at tick precision; the candidate is compared at it.
                    let stored = history::stored_form(
                        &DailySummaryRevision::new(head.revision(), candidate.clone())
                            .expect("a stored head has a positive revision"),
                    )
                    .map_err(WeeklyCreditError::from)?;
                    if !has_equal_durable_material(stored.snapshot(), head.snapshot()) {
                        return Err(WeeklyError::FingerprintCollision(date));
                    }
                    head.revision()
                }
                _ => {
                    let revision = revision_vector
                        .revision(date)
                        .expect("the plan covers weekdays of its week")
                        .checked_add(1)
                        .expect("revisions stay far below the integer range");
                    revisions_to_append.push(
                        DailySummaryRevision::new(revision, candidate)
                            .expect("the next revision is positive"),
                    );
                    revision
                }
            };
            selected_revisions.insert(date, selected);
            if Some(date) == plan.summary_date() {
                current_summary = Some(summary);
            }
        }

        // History holds weekdays only, so a weekend is summarized against its nominal target.
        if let Some(today) = plan.summary_date().filter(|today| is_weekend(*today)) {
            let weekend = self.accounting.calculate(today, &context)?;
            current_summary = Some(
                summaries::build_nominal(&weekend.summary_input)
                    .expect("a summary of a domain calculation is consistent"),
            );
        }

        let ledger = build_ledger(
            &credit_results,
            &selected_revisions,
            &friday_window,
            context.observed_at,
            state.ledger(),
        )
        .map_err(WeeklyError::Ledger)?;
        self.store
            .commit(monday, &revision_vector, &revisions_to_append, &ledger)?;
        Ok(current_summary)
    }

    /// The current week runs in the captured context. A past week is replayed with today's
    /// settings but in the zone its summaries were stored in, so its windows do not move.
    fn resolve_context(
        &self,
        plan: WeekPlan,
        state: &WeeklyCreditState,
        captured: CalculationContext,
    ) -> Result<CalculationContext, WeeklyError> {
        if let WeekPlan::CurrentObservedPrefix { .. } = plan {
            return Ok(captured);
        }
        let zones: HashSet<&str> = state
            .latest_revisions()
            .iter()
            .map(|revision| revision.snapshot().workday().time_zone_id.as_str())
            .collect();
        let [time_zone_id] = zones.into_iter().collect::<Vec<_>>()[..] else {
            return Err(WeeklyError::MixedTimeZones(plan.monday()));
        };
        let time_zone = self
            .accounting
            .resolve_time_zone(time_zone_id)
            .map_err(WeeklyError::UnknownTimeZone)?;
        Ok(CalculationContext {
            time_zone_id: time_zone_id.to_owned(),
            time_zone,
            ..captured
        })
    }
}

/// Refuses to recalculate when the clock went back: a stored head of a day after today, or one
/// computed after now, would otherwise be dropped or overwritten with older material. Runs
/// before any source read.
fn validate_clock_has_not_moved_backward(
    state: &WeeklyCreditState,
    observed_dates: &[Date],
    observed_at: Timestamp,
) -> Result<(), WeeklyError> {
    let heads = state.latest_revisions();
    if let Some(later) = heads
        .iter()
        .find(|revision| !observed_dates.contains(&revision.workday_date()))
    {
        return Err(WeeklyError::ClockRollbackDropsWorkday(later.workday_date()));
    }
    if let Some(future) = heads
        .iter()
        .find(|revision| revision.snapshot().computed_at() > observed_at)
    {
        return Err(WeeklyError::ClockRollbackPrecedesMaterial(
            future.workday_date(),
        ));
    }
    Ok(())
}

/// Every stored value but the computation instant and the fingerprint.
fn has_equal_durable_material(left: &DailySummarySnapshot, right: &DailySummarySnapshot) -> bool {
    left.workday() == right.workday()
        && left.credited_work() == right.credited_work()
        && left.excluded_work() == right.excluded_work()
        && left.unknown_work() == right.unknown_work()
        && left.nominal_target() == right.nominal_target()
        && left.effective_target() == right.effective_target()
        && left.carry_available() == right.carry_available()
        && left.carry_applied() == right.carry_applied()
        && left.carry_earned() == right.carry_earned()
        && left.carry_expired() == right.carry_expired()
        && left.carry_debt() == right.carry_debt()
        && left.debt_pending() == right.debt_pending()
        && left.projected_departure() == right.projected_departure()
        && left.indicator_state() == right.indicator_state()
        && left.source_quality() == right.source_quality()
        && left.has_pending_review() == right.has_pending_review()
        && left.breakdown() == right.breakdown()
}

fn window_end(calculation: &WorkdayCalculation) -> Timestamp {
    calculation
        .summary_input
        .accounting()
        .timeline()
        .window()
        .end
}

fn is_weekend(date: Date) -> bool {
    matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
}

pub(crate) fn start_of_week(date: Date) -> Date {
    days_after(date, -i64::from(date.weekday().to_monday_zero_offset()))
}

fn days_after(date: Date, days: i64) -> Date {
    date.checked_add(jiff::Span::new().days(days))
        .expect("workweeks are within the calendar")
}

#[cfg(test)]
mod tests;
