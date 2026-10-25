//! Exactly-once notification delivery, ported from
//! `Notifications/NotificationReceiptRepository.cs`.

use std::collections::BTreeSet;

use bientot_domain::notifications::NotificationTrigger;
use bientot_domain::time::LocalWorkday;
use jiff::Timestamp;
use rusqlite::{Connection, TransactionBehavior, named_params};

use crate::codec::{format_date, format_instant};
use crate::row::{StorageError, storage_error_conversions};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationClaim {
    pub workday: LocalWorkday,
    pub trigger: NotificationTrigger,
    pub eligible_at: Timestamp,
}

#[derive(Debug)]
pub enum NotificationReceiptError {
    Storage(StorageError),
    BlankDeliveryId,
    /// One reservation covers the claims of a single workday.
    MixedWorkdays,
}

impl std::fmt::Display for NotificationReceiptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::BlankDeliveryId => f.write_str("The combined delivery ID cannot be blank."),
            Self::MixedWorkdays => f.write_str("Claims must belong to one workday."),
        }
    }
}

impl std::error::Error for NotificationReceiptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

storage_error_conversions!(NotificationReceiptError);

fn trigger_storage(trigger: NotificationTrigger) -> &'static str {
    match trigger {
        NotificationTrigger::CompletionApproach => "completion-approach",
        NotificationTrigger::LatestDepartureApproach => "latest-departure-approach",
        NotificationTrigger::DayComplete => "day-complete",
    }
}

pub struct NotificationReceiptRepository<'a> {
    connection: &'a Connection,
}

impl<'a> NotificationReceiptRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Reserves each trigger not yet reserved for the workday under `combined_delivery_id`, and
    /// returns the triggers this call reserved. A trigger claimed twice counts once, with its
    /// first claim.
    pub fn try_reserve(
        &self,
        claims: &[NotificationClaim],
        combined_delivery_id: &str,
        reserved_at: Timestamp,
    ) -> Result<BTreeSet<NotificationTrigger>, NotificationReceiptError> {
        if combined_delivery_id.trim().is_empty() {
            return Err(NotificationReceiptError::BlankDeliveryId);
        }
        let Some(first) = claims.first() else {
            return Ok(BTreeSet::new());
        };
        if claims.iter().any(|claim| claim.workday != first.workday) {
            return Err(NotificationReceiptError::MixedWorkdays);
        }

        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let mut claimed = BTreeSet::new();
        let mut reserved = BTreeSet::new();
        for claim in claims.iter().filter(|claim| claimed.insert(claim.trigger)) {
            let inserted = transaction.execute(
                "INSERT OR IGNORE INTO NotificationReceipt
    (WorkdayDate, Trigger, EligibleAtUtc, ReservedAtUtc, CombinedDeliveryId)
VALUES ($date, $trigger, $eligible, $reserved, $deliveryId);",
                named_params! {
                    "$date": format_date(claim.workday.date),
                    "$trigger": trigger_storage(claim.trigger),
                    "$eligible": format_instant(claim.eligible_at),
                    "$reserved": format_instant(reserved_at),
                    "$deliveryId": combined_delivery_id,
                },
            )?;
            if inserted == 1 {
                reserved.insert(claim.trigger);
            }
        }
        transaction.commit()?;
        Ok(reserved)
    }
}

#[cfg(test)]
mod tests;
