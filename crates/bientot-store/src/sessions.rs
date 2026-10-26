//! The workday transitions of the three state lanes, ported from
//! `Sessions/SessionEventRepository.cs`.

use std::collections::HashSet;

use bientot_domain::accounting::{
    AutomaticOrigin, OperationKey, PowerState, Recorded, RecordedTransition,
    SESSION_HISTORY_SOURCE_ID, SessionState, TrackingState, WorkdayEventHistory, WorkdayEventId,
    WorkdayStateAnchors, WorkdayTransition, WorkdayTransitionKind,
};
use bientot_domain::time::{Clock, WorkdayWindow};
use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::codec::{format_instant, truncate_to_tick};
use crate::row::{InvalidStoredValue, StorageError, StoredRow, collect, storage_error_conversions};

const COLUMNS: &str = "Id, OccurredAtUtc, TimezoneId, Kind, Source, CreatedAtUtc, OperationKey";
/// Unknown observations sort before a decisive state recorded at the same instant.
const STATE_PRIORITY: &str =
    "CASE WHEN Kind IN ('session-unknown', 'power-unknown') THEN 0 ELSE 1 END";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordTransitionOutcome {
    Inserted,
    AlreadyRecorded,
    /// Stored as a receipt for its operation key, but its lane was already in that state.
    NoStateChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordTransitionResult {
    pub outcome: RecordTransitionOutcome,
    pub authoritative_id: WorkdayEventId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicWorkdayTransition {
    pub operation_key: OperationKey,
    pub transition: WorkdayTransition,
}

#[derive(Debug)]
pub enum SessionEventError {
    Storage(StorageError),
    /// The operation key already identifies a different transition.
    OperationConflict(OperationKey),
    /// One batch names the same operation key twice.
    DuplicateOperationKey(OperationKey),
    EmptyWindow,
}

impl std::fmt::Display for SessionEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::OperationConflict(key) => write!(
                f,
                "Operation key '{key}' already identifies a different workday transition."
            ),
            Self::DuplicateOperationKey(key) => write!(
                f,
                "Operation key '{key}' occurs more than once in an atomic transition batch."
            ),
            Self::EmptyWindow => f.write_str("The workday window must have a positive duration."),
        }
    }
}

impl std::error::Error for SessionEventError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

storage_error_conversions!(SessionEventError);

/// The `Source` column: which lane a transition belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    WindowsSession,
    WindowsPower,
    User,
}

impl Lane {
    const ALL: [Self; 3] = [Self::WindowsSession, Self::WindowsPower, Self::User];

    fn storage(self) -> &'static str {
        match self {
            Self::WindowsSession => "windows-session",
            Self::WindowsPower => "windows-power",
            Self::User => "user",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|lane| lane.storage() == text)
    }
}

/// Every transition the ledger stores, with its lane and `Kind` text.
const KINDS: [(WorkdayTransitionKind, Lane, &str); 9] = [
    (
        WorkdayTransitionKind::SessionObserved(SessionState::Locked),
        Lane::WindowsSession,
        "locked",
    ),
    (
        WorkdayTransitionKind::SessionObserved(SessionState::Unlocked),
        Lane::WindowsSession,
        "unlocked",
    ),
    (
        WorkdayTransitionKind::SessionObserved(SessionState::Unknown),
        Lane::WindowsSession,
        "session-unknown",
    ),
    (
        WorkdayTransitionKind::PowerObserved(PowerState::Suspended),
        Lane::WindowsPower,
        "suspend",
    ),
    (
        WorkdayTransitionKind::PowerObserved(PowerState::Active),
        Lane::WindowsPower,
        "resume",
    ),
    (
        WorkdayTransitionKind::PowerObserved(PowerState::Unknown),
        Lane::WindowsPower,
        "power-unknown",
    ),
    (
        WorkdayTransitionKind::TrackingChanged(TrackingState::Paused),
        Lane::User,
        "tracking-paused",
    ),
    (
        WorkdayTransitionKind::TrackingChanged(TrackingState::Running),
        Lane::User,
        "tracking-resumed",
    ),
    (
        WorkdayTransitionKind::TrackingChanged(TrackingState::Finished),
        Lane::User,
        "day-finished",
    ),
];

fn lane_and_kind(kind: WorkdayTransitionKind) -> (Lane, &'static str) {
    KINDS
        .into_iter()
        .find(|(stored, ..)| *stored == kind)
        .map(|(_, lane, text)| (lane, text))
        .expect("every transition kind has a stored form")
}

/// The operation keys of the session transitions read from the Windows logs after the fact;
/// they are recorded with [`SESSION_HISTORY_SOURCE_ID`] as their origin.
pub const SESSION_HISTORY_KEY_PREFIX: &str = "session-history:";

/// A transition as the ledger holds it, its instant truncated to .NET ticks.
#[derive(Clone, Debug)]
struct StoredTransition {
    id: WorkdayEventId,
    transition: WorkdayTransition,
    /// Read from the Windows logs rather than observed.
    from_history: bool,
}

impl StoredTransition {
    fn lane(&self) -> Lane {
        lane_and_kind(self.transition.kind()).0
    }

    fn to_recorded(&self) -> RecordedTransition {
        let source_id = if self.from_history {
            SESSION_HISTORY_SOURCE_ID
        } else {
            self.lane().storage()
        };
        let origin = AutomaticOrigin::new(source_id).expect("source names are not blank");
        let at = self.transition.occurred_at();
        match self.transition.kind() {
            WorkdayTransitionKind::SessionObserved(state) => {
                Recorded::new(self.id, at, state, origin).into()
            }
            WorkdayTransitionKind::PowerObserved(state) => {
                Recorded::new(self.id, at, state, origin).into()
            }
            WorkdayTransitionKind::TrackingChanged(state) => {
                Recorded::new(self.id, at, state, origin).into()
            }
        }
    }
}

/// The candidate as it will be stored: .NET instants have 100 ns ticks.
fn stored_form(transition: &WorkdayTransition) -> WorkdayTransition {
    WorkdayTransition::new(
        truncate_to_tick(transition.occurred_at()),
        transition.time_zone_id(),
        transition.kind(),
    )
    .expect("the time zone of a transition is already canonical")
}

pub struct SessionEventRepository<'a> {
    connection: &'a Connection,
    clock: &'a dyn Clock,
}

impl<'a> SessionEventRepository<'a> {
    pub fn new(connection: &'a Connection, clock: &'a dyn Clock) -> Self {
        Self { connection, clock }
    }

    pub fn record(
        &self,
        operation_key: OperationKey,
        transition: WorkdayTransition,
    ) -> Result<RecordTransitionResult, SessionEventError> {
        let results = self.record_atomically(&[AtomicWorkdayTransition {
            operation_key,
            transition,
        }])?;
        Ok(results[0])
    }

    /// Records every transition or none. A transition whose operation key is already stored is
    /// not recorded again; one that leaves its lane in the state it was in at that instant is
    /// still stored, as a receipt for its key.
    pub fn record_atomically(
        &self,
        transitions: &[AtomicWorkdayTransition],
    ) -> Result<Vec<RecordTransitionResult>, SessionEventError> {
        if transitions.is_empty() {
            return Ok(Vec::new());
        }
        let mut keys = HashSet::new();
        if let Some(duplicate) = transitions
            .iter()
            .find(|item| !keys.insert(&item.operation_key))
        {
            return Err(SessionEventError::DuplicateOperationKey(
                duplicate.operation_key.clone(),
            ));
        }

        let created_at = format_instant(self.clock.now());
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        let mut results = Vec::with_capacity(transitions.len());
        for item in transitions {
            let transition = stored_form(&item.transition);
            if let Some(existing) = find_by_operation_key(&transaction, &item.operation_key)? {
                if existing.transition != transition {
                    return Err(SessionEventError::OperationConflict(
                        item.operation_key.clone(),
                    ));
                }
                results.push(RecordTransitionResult {
                    outcome: RecordTransitionOutcome::AlreadyRecorded,
                    authoritative_id: existing.id,
                });
                continue;
            }

            let predecessor = read_predecessor_in_lane(&transaction, &transition)?;
            let outcome = if predecessor
                .is_some_and(|predecessor| predecessor.transition.kind() == transition.kind())
            {
                RecordTransitionOutcome::NoStateChange
            } else {
                RecordTransitionOutcome::Inserted
            };
            let id = insert(&transaction, &item.operation_key, &transition, &created_at)?;
            results.push(RecordTransitionResult {
                outcome,
                authoritative_id: id,
            });
        }
        transaction.commit()?;
        Ok(results)
    }

    /// The transition stored under `operation_key`, if any.
    pub fn find(
        &self,
        operation_key: &OperationKey,
    ) -> Result<Option<RecordedTransition>, SessionEventError> {
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Deferred)?;
        let found = find_by_operation_key(&transaction, operation_key)?;
        Ok(found.map(|stored| stored.to_recorded()))
    }

    /// The transitions inside `window`, and for each lane the transition that started the state
    /// it was in when the window began.
    pub fn read_for_accounting(
        &self,
        window: &WorkdayWindow,
    ) -> Result<WorkdayEventHistory, SessionEventError> {
        if window.end <= window.start {
            return Err(SessionEventError::EmptyWindow);
        }
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Deferred)?;
        let [session, power, tracking] =
            Lane::ALL.map(|lane| read_anchor(&transaction, lane, window.start));
        let anchors = WorkdayStateAnchors {
            session: session?.map(|anchor| match anchor.to_recorded() {
                RecordedTransition::Session(recorded) => recorded,
                _ => unreachable!("the session anchor is read from the session lane"),
            }),
            power: power?.map(|anchor| match anchor.to_recorded() {
                RecordedTransition::Power(recorded) => recorded,
                _ => unreachable!("the power anchor is read from the power lane"),
            }),
            tracking: tracking?.map(|anchor| match anchor.to_recorded() {
                RecordedTransition::Tracking(recorded) => recorded,
                _ => unreachable!("the tracking anchor is read from the user lane"),
            }),
        };
        let in_window = read_in_window(&transaction, window)?;
        transaction.commit()?;
        Ok(WorkdayEventHistory::new(
            anchors,
            in_window.iter().map(StoredTransition::to_recorded),
        ))
    }
}

fn find_by_operation_key(
    transaction: &Connection,
    operation_key: &OperationKey,
) -> Result<Option<StoredTransition>, SessionEventError> {
    let found = transaction
        .query_row(
            &format!("SELECT {COLUMNS} FROM SessionEvent WHERE OperationKey = $operationKey;"),
            named_params! { "$operationKey": operation_key.as_str() },
            |row| Ok(parse(&StoredRow::new(row, "SessionEvent"))),
        )
        .optional()?;
    Ok(found.transpose()?)
}

fn read_predecessor_in_lane(
    transaction: &Connection,
    transition: &WorkdayTransition,
) -> Result<Option<StoredTransition>, SessionEventError> {
    let lane = lane_and_kind(transition.kind()).0;
    let found = transaction
        .query_row(
            &format!(
                "SELECT {COLUMNS}
FROM SessionEvent
WHERE Source = $source
  AND OccurredAtUtc <= $occurredAt
ORDER BY OccurredAtUtc DESC, {STATE_PRIORITY} DESC, Id DESC
LIMIT 1;"
            ),
            named_params! {
                "$source": lane.storage(),
                "$occurredAt": format_instant(transition.occurred_at()),
            },
            |row| Ok(parse(&StoredRow::new(row, "SessionEvent"))),
        )
        .optional()?;
    Ok(found.transpose()?)
}

/// The earliest transition of the run of same-state transitions that ends last before `before`.
fn read_anchor(
    transaction: &Connection,
    lane: Lane,
    before: Timestamp,
) -> Result<Option<StoredTransition>, SessionEventError> {
    let mut statement = transaction.prepare(&format!(
        "SELECT {COLUMNS}
FROM SessionEvent
WHERE Source = $source
  AND OccurredAtUtc < $before
ORDER BY OccurredAtUtc DESC, {STATE_PRIORITY} DESC, Id DESC"
    ))?;
    let mut rows = statement.query(named_params! {
        "$source": lane.storage(),
        "$before": format_instant(before),
    })?;
    let mut anchor: Option<StoredTransition> = None;
    while let Some(row) = rows.next()? {
        let candidate = parse(&StoredRow::new(row, "SessionEvent"))?;
        if anchor
            .as_ref()
            .is_some_and(|anchor| anchor.transition.kind() != candidate.transition.kind())
        {
            break;
        }
        anchor = Some(candidate);
    }
    Ok(anchor)
}

fn read_in_window(
    transaction: &Connection,
    window: &WorkdayWindow,
) -> Result<Vec<StoredTransition>, SessionEventError> {
    let mut statement = transaction.prepare(&format!(
        "SELECT {COLUMNS}
FROM SessionEvent
WHERE OccurredAtUtc >= $start
  AND OccurredAtUtc < $end
ORDER BY OccurredAtUtc, {STATE_PRIORITY}, Id;"
    ))?;
    let rows = statement.query_map(
        named_params! {
            "$start": format_instant(window.start),
            "$end": format_instant(window.end),
        },
        |row| Ok(parse(&StoredRow::new(row, "SessionEvent"))),
    )?;
    collect(rows)
}

fn insert(
    transaction: &Connection,
    operation_key: &OperationKey,
    transition: &WorkdayTransition,
    created_at: &str,
) -> Result<WorkdayEventId, SessionEventError> {
    let (lane, kind) = lane_and_kind(transition.kind());
    let id = transaction.query_row(
        "INSERT INTO SessionEvent (
    OccurredAtUtc,
    TimezoneId,
    Kind,
    Source,
    CreatedAtUtc,
    OperationKey)
VALUES (
    $occurredAtUtc,
    $timezoneId,
    $kind,
    $source,
    $createdAtUtc,
    $operationKey)
RETURNING Id;",
        named_params! {
            "$occurredAtUtc": format_instant(transition.occurred_at()),
            "$timezoneId": transition.time_zone_id(),
            "$kind": kind,
            "$source": lane.storage(),
            "$createdAtUtc": created_at,
            "$operationKey": operation_key.as_str(),
        },
        |row| row.get(0),
    )?;
    Ok(WorkdayEventId(id))
}

fn parse(row: &StoredRow) -> Result<StoredTransition, InvalidStoredValue> {
    let id = row.integer("Id")?;
    if id <= 0 {
        return Err(row.invalid("the event ID must be positive"));
    }
    let occurred_at = row.instant("OccurredAtUtc")?;
    let time_zone_id = row.time_zone_id("TimezoneId")?;
    let kind = row.text("Kind")?;
    let lane = Lane::parse(row.text("Source")?)
        .ok_or_else(|| row.invalid("Source is not a recognized event source"))?;
    row.instant("CreatedAtUtc")?;
    let key = row.optional_text("OperationKey")?;
    if let Some(key) = key
        && !OperationKey::new(key).is_ok_and(|parsed| parsed.as_str() == key)
    {
        return Err(row.invalid("OperationKey is not canonical"));
    }
    let from_history = key.is_some_and(|key| key.starts_with(SESSION_HISTORY_KEY_PREFIX));
    let kind = KINDS
        .into_iter()
        .find(|&(_, stored_lane, text)| stored_lane == lane && text == kind)
        .map(|(kind, ..)| kind)
        .ok_or_else(|| row.invalid("Kind and Source do not form a valid event lane transition"))?;
    let transition = WorkdayTransition::new(occurred_at, time_zone_id, kind)
        .expect("the time zone was checked to be canonical");
    Ok(StoredTransition {
        id: WorkdayEventId(id),
        transition,
        from_history,
    })
}

#[cfg(test)]
mod tests;
