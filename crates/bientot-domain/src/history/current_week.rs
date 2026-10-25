use std::collections::HashMap;

use jiff::civil::Date;

use super::DailySummaryRevision;
use crate::time::days_after;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentWeekDay {
    pub date: Date,
    pub latest_revision: Option<DailySummaryRevision>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidCurrentWeek {
    RevisionOutsideWeek,
    DuplicateDay,
}

impl std::fmt::Display for InvalidCurrentWeek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RevisionOutsideWeek => {
                "current-week revisions must belong to the resolved Monday-to-Friday week"
            }
            Self::DuplicateDay => "current-week revisions must hold one revision per day",
        })
    }
}

impl std::error::Error for InvalidCurrentWeek {}

/// The Monday-to-Friday week containing a date, weekends resolving to the week they end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentWeek {
    week_start_date: Date,
    days: [CurrentWeekDay; 5],
}

impl CurrentWeek {
    pub fn new(
        current_date: Date,
        latest_revisions: impl IntoIterator<Item = DailySummaryRevision>,
    ) -> Result<Self, InvalidCurrentWeek> {
        let week_start_date = days_after(
            current_date,
            -i64::from(current_date.weekday().to_monday_zero_offset()),
        );
        let week_end_date = days_after(week_start_date, 4);
        let revisions: Vec<_> = latest_revisions.into_iter().collect();
        if revisions.iter().any(|revision| {
            revision.workday_date() < week_start_date || revision.workday_date() > week_end_date
        }) {
            return Err(InvalidCurrentWeek::RevisionOutsideWeek);
        }
        let mut by_date = HashMap::with_capacity(revisions.len());
        for revision in revisions {
            if by_date.insert(revision.workday_date(), revision).is_some() {
                return Err(InvalidCurrentWeek::DuplicateDay);
            }
        }
        let days = std::array::from_fn(|offset| {
            let date = days_after(week_start_date, offset as i64);
            CurrentWeekDay {
                date,
                latest_revision: by_date.remove(&date),
            }
        });
        Ok(Self {
            week_start_date,
            days,
        })
    }

    pub fn week_start_date(&self) -> Date {
        self.week_start_date
    }

    pub fn days(&self) -> &[CurrentWeekDay] {
        &self.days
    }
}

#[cfg(test)]
mod tests;
