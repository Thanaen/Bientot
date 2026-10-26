//! Reading Windows event logs: the records a query collects and their event XML. The history
//! readers interpret the records; only the query itself is Windows only.

#[cfg(windows)]
pub(crate) mod native;
pub(crate) mod xml;

/// The matching records a read collected, newest first, as event XML.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectedRecords {
    Complete(Vec<String>),
    /// The log held more than the record limit; the records read are dropped.
    Truncated,
}

/// The log could not be queried or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogUnavailable;

/// Reads up to `record_limit` records, then one more to tell the end of the log from the cap.
pub fn collect<E>(
    mut read_next: impl FnMut() -> Result<Option<String>, E>,
    record_limit: usize,
) -> Result<CollectedRecords, E> {
    let mut records = Vec::new();
    for _ in 0..record_limit {
        match read_next()? {
            Some(record) => records.push(record),
            None => return Ok(CollectedRecords::Complete(records)),
        }
    }
    Ok(match read_next()? {
        Some(_) => CollectedRecords::Truncated,
        None => CollectedRecords::Complete(records),
    })
}
