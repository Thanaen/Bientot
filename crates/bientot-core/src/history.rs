//! The history of the current week, ported from `Services/HistoryService.cs`.
//!
//! `Services/HistoryRefreshBarrier.cs` is not ported: it awaits the week's recalculation before
//! publishing the current day, which synchronous callers get from the order of their two calls.

use bientot_domain::history::CurrentWeek;
use bientot_domain::time::{Clock, TimeZoneProvider, WorkdayCalendar};
use bientot_store::history::{DailySummaryError, DailySummaryRepository};

/// Reads the stored summaries of the workweek containing the current local day.
pub struct HistoryService<'a> {
    summaries: DailySummaryRepository<'a>,
    clock: &'a dyn Clock,
    zones: &'a dyn TimeZoneProvider,
}

impl<'a> HistoryService<'a> {
    pub fn new(
        summaries: DailySummaryRepository<'a>,
        clock: &'a dyn Clock,
        zones: &'a dyn TimeZoneProvider,
    ) -> Self {
        Self {
            summaries,
            clock,
            zones,
        }
    }

    /// The five weekdays of the current week, each with its latest stored revision if it has
    /// one. A weekend shows the week it ends. A stored row that cannot be read fails the call
    /// rather than showing as a missing day.
    pub fn current_week(&self) -> Result<CurrentWeek, DailySummaryError> {
        let today = WorkdayCalendar::new(self.zones)
            .workday(self.clock.now())
            .date;
        let revisions = self.summaries.read_latest_week(today)?;
        Ok(CurrentWeek::new(today, revisions)
            .expect("the ledger returns one latest revision per weekday of the week asked for"))
    }
}

#[cfg(test)]
mod tests;
