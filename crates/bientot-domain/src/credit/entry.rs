use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};

use super::is_workday;
use crate::accounting::BlankIdentifier;
use crate::accounting::model::non_blank;
use crate::time::days_after;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CreditLedgerEntryId(String);

impl CreditLedgerEntryId {
    pub fn new(value: impl Into<String>) -> Result<Self, BlankIdentifier> {
        non_blank(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CreditLedgerEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CreditLedgerEntryKind {
    Earned,
    Applied,
    Expired,
    Reversed,
}

/// The earned lot a movement draws on: the day it was earned and that day's durable revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CreditOrigin {
    pub workday_date: Date,
    pub revision: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidLedgerEntry {
    NotAWorkday,
    NonPositiveAmount,
    OriginNotAWorkday,
    NonPositiveOriginRevision,
}

impl std::fmt::Display for InvalidLedgerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotAWorkday => "a credit ledger entry must belong to a Monday-to-Friday workday",
            Self::NonPositiveAmount => "credit magnitude must be positive",
            Self::OriginNotAWorkday => "credit provenance must identify a Monday-to-Friday workday",
            Self::NonPositiveOriginRevision => "credit provenance revision must be positive",
        })
    }
}

impl std::error::Error for InvalidLedgerEntry {}

/// One movement of a credit lot: earned on its origin day, then applied or expired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreditLedgerEntry {
    id: CreditLedgerEntryId,
    workday_date: Date,
    kind: CreditLedgerEntryKind,
    amount: SignedDuration,
    origin: CreditOrigin,
    created_at: Timestamp,
}

impl CreditLedgerEntry {
    pub fn new(
        id: CreditLedgerEntryId,
        workday_date: Date,
        kind: CreditLedgerEntryKind,
        amount: SignedDuration,
        origin: CreditOrigin,
        created_at: Timestamp,
    ) -> Result<Self, InvalidLedgerEntry> {
        if !is_workday(workday_date) {
            return Err(InvalidLedgerEntry::NotAWorkday);
        }
        if !amount.is_positive() {
            return Err(InvalidLedgerEntry::NonPositiveAmount);
        }
        if !is_workday(origin.workday_date) {
            return Err(InvalidLedgerEntry::OriginNotAWorkday);
        }
        if origin.revision <= 0 {
            return Err(InvalidLedgerEntry::NonPositiveOriginRevision);
        }
        Ok(Self {
            id,
            workday_date,
            kind,
            amount,
            origin,
            created_at,
        })
    }

    pub fn id(&self) -> &CreditLedgerEntryId {
        &self.id
    }

    pub fn workday_date(&self) -> Date {
        self.workday_date
    }

    pub fn week_start_date(&self) -> Date {
        days_after(
            self.workday_date,
            -i64::from(self.workday_date.weekday().to_monday_zero_offset()),
        )
    }

    pub fn kind(&self) -> CreditLedgerEntryKind {
        self.kind
    }

    pub fn amount(&self) -> SignedDuration {
        self.amount
    }

    pub fn origin(&self) -> CreditOrigin {
        self.origin
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn entry(
        workday: Date,
        amount: SignedDuration,
        origin: CreditOrigin,
    ) -> Result<CreditLedgerEntry, InvalidLedgerEntry> {
        CreditLedgerEntry::new(
            CreditLedgerEntryId::new("entry").unwrap(),
            workday,
            CreditLedgerEntryKind::Earned,
            amount,
            origin,
            Timestamp::UNIX_EPOCH,
        )
    }

    #[test]
    fn an_entry_belongs_to_the_monday_of_its_workweek() {
        let friday = date(2026, 9, 11);

        let entry = entry(
            friday,
            SignedDuration::from_mins(1),
            CreditOrigin {
                workday_date: friday,
                revision: 1,
            },
        )
        .unwrap();

        assert_eq!(entry.week_start_date(), date(2026, 9, 7));
    }

    #[test]
    fn entries_reject_weekends_empty_amounts_and_unknown_provenance() {
        let monday = date(2026, 9, 7);
        let sunday = date(2026, 9, 6);
        let minute = SignedDuration::from_mins(1);

        assert_eq!(
            entry(
                sunday,
                minute,
                CreditOrigin {
                    workday_date: monday,
                    revision: 1
                }
            ),
            Err(InvalidLedgerEntry::NotAWorkday)
        );
        assert_eq!(
            entry(
                monday,
                SignedDuration::ZERO,
                CreditOrigin {
                    workday_date: monday,
                    revision: 1
                }
            ),
            Err(InvalidLedgerEntry::NonPositiveAmount)
        );
        assert_eq!(
            entry(
                monday,
                minute,
                CreditOrigin {
                    workday_date: sunday,
                    revision: 1
                }
            ),
            Err(InvalidLedgerEntry::OriginNotAWorkday)
        );
        assert_eq!(
            entry(
                monday,
                minute,
                CreditOrigin {
                    workday_date: monday,
                    revision: 0
                }
            ),
            Err(InvalidLedgerEntry::NonPositiveOriginRevision)
        );
        assert_eq!(CreditLedgerEntryId::new(" "), Err(BlankIdentifier));
    }
}
