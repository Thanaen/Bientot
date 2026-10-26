//! The System log query of the .NET `SystemEventLogSource`.

use crate::event_log::{CollectedRecords, LogUnavailable, native};

use super::{SystemEventLog, SystemLogUnavailable};

const QUERY: &str = "*[System[((Provider[@Name='Microsoft-Windows-Kernel-General']) and (EventID=12 or EventID=13)) or ((Provider[@Name='Microsoft-Windows-Kernel-Power']) and (EventID=42 or EventID=107))]]";

/// The `System` log of this computer, read newest first.
pub struct SystemLog;

impl SystemEventLog for SystemLog {
    fn read(&self, record_limit: usize) -> Result<CollectedRecords, SystemLogUnavailable> {
        native::read("System", QUERY, record_limit).map_err(|LogUnavailable| SystemLogUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use bientot_domain::intervals::UtcInterval;
    use jiff::Timestamp;

    use super::*;
    use crate::power_history::{PowerHistoryRead, PowerHistoryReader, RECORD_LIMIT};

    #[test]
    fn the_system_log_reads_and_every_matching_record_parses() {
        let read = PowerHistoryReader::new(SystemLog, RECORD_LIMIT)
            .read(UtcInterval::new(Timestamp::UNIX_EPOCH, Timestamp::now()).unwrap());

        assert!(
            matches!(
                read,
                PowerHistoryRead::Complete(_) | PowerHistoryRead::Truncated
            ),
            "{read:?}"
        );
    }
}
