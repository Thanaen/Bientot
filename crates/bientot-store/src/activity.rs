//! The built-in activity spans and the history of the selected activity source, ported from
//! `Activity/ActivitySpanRepository.cs` and `Activity/ActivitySourceSelectionRepository.cs`.

use bientot_domain::sources::{ActivitySourceKind, ActivitySourceSelection, IdleSpan, IdleState};
use bientot_domain::time::Clock;
use jiff::Timestamp;
use rusqlite::{Connection, TransactionBehavior, named_params};

use crate::codec::format_instant;
use crate::row::{InvalidStoredValue, StorageError, StoredRow, collect, storage_error_conversions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActivitySpanId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredActivitySpan {
    pub id: ActivitySpanId,
    pub span: IdleSpan,
}

#[derive(Debug)]
pub enum ActivityError {
    Storage(StorageError),
    /// A read interval must end after it starts.
    EmptyInterval,
}

impl std::fmt::Display for ActivityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::EmptyInterval => f.write_str("The read interval must have a positive duration."),
        }
    }
}

impl std::error::Error for ActivityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::EmptyInterval => None,
        }
    }
}

storage_error_conversions!(ActivityError);

fn idle_state_storage(state: IdleState) -> &'static str {
    match state {
        IdleState::Active => "active",
        IdleState::Idle => "idle",
    }
}

fn source_storage(source: ActivitySourceKind) -> &'static str {
    match source {
        ActivitySourceKind::BuiltIn => "built-in",
        ActivitySourceKind::ActivityWatch => "activitywatch",
    }
}

pub struct ActivitySpanRepository<'a> {
    connection: &'a Connection,
    clock: &'a dyn Clock,
}

impl<'a> ActivitySpanRepository<'a> {
    pub fn new(connection: &'a Connection, clock: &'a dyn Clock) -> Self {
        Self { connection, clock }
    }

    /// Appends `span`. When `closes` is set, that span is settled up to the start of `span` in
    /// the same transaction. Appending a span already stored only advances its settled boundary
    /// and returns its id.
    pub fn append(
        &self,
        span: IdleSpan,
        closes: Option<ActivitySpanId>,
    ) -> Result<ActivitySpanId, ActivityError> {
        let transaction = crate::database::scope(self.connection, TransactionBehavior::Immediate)?;
        if let Some(closed) = closes {
            advance(&transaction, closed, span.start())?;
        }
        let state = idle_state_storage(span.state());
        let start = format_instant(span.start());
        transaction.execute(
            "INSERT INTO ActivitySpan (State, StartUtc, SettledThroughUtc, CreatedAtUtc)
VALUES ($state, $start, $settled, $created)
ON CONFLICT (StartUtc, State) DO NOTHING;",
            named_params! {
                "$state": state,
                "$start": start,
                "$settled": format_instant(span.settled_through()),
                "$created": format_instant(self.clock.now()),
            },
        )?;
        let id = ActivitySpanId(transaction.query_row(
            "SELECT Id FROM ActivitySpan WHERE StartUtc = $start AND State = $state;",
            named_params! { "$state": state, "$start": start },
            |row| row.get(0),
        )?);
        advance(&transaction, id, span.settled_through())?;
        transaction.commit()?;
        Ok(id)
    }

    /// Moves the stored settled boundary forward; an older value leaves it unchanged.
    pub fn advance_settled(
        &self,
        id: ActivitySpanId,
        settled_through: Timestamp,
    ) -> Result<(), ActivityError> {
        advance(self.connection, id, settled_through)
    }

    /// The spans whose settled part overlaps `[start, end)`, by start then id.
    pub fn read_settled(
        &self,
        start: Timestamp,
        end: Timestamp,
    ) -> Result<Vec<StoredActivitySpan>, ActivityError> {
        if end <= start {
            return Err(ActivityError::EmptyInterval);
        }
        let mut statement = self.connection.prepare(
            "SELECT Id, State, StartUtc, SettledThroughUtc
FROM ActivitySpan
WHERE StartUtc < $end AND SettledThroughUtc > $start
ORDER BY StartUtc, Id;",
        )?;
        let rows = statement.query_map(
            named_params! {
                "$start": format_instant(start),
                "$end": format_instant(end),
            },
            |row| Ok(parse_span(&StoredRow::new(row, "ActivitySpan"))),
        )?;
        collect(rows)
    }
}

fn advance(
    connection: &Connection,
    id: ActivitySpanId,
    settled_through: Timestamp,
) -> Result<(), ActivityError> {
    connection.execute(
        "UPDATE ActivitySpan
SET SettledThroughUtc = $settled
WHERE Id = $id AND SettledThroughUtc < $settled;",
        named_params! {
            "$id": id.0,
            "$settled": format_instant(settled_through),
        },
    )?;
    Ok(())
}

fn parse_span(row: &StoredRow) -> Result<StoredActivitySpan, InvalidStoredValue> {
    let id = ActivitySpanId(row.integer("Id")?);
    let state = match row.text("State")? {
        "active" => IdleState::Active,
        "idle" => IdleState::Idle,
        state => {
            return Err(row.invalid(format!(
                "stored activity span state '{state}' is not recognized"
            )));
        }
    };
    let span = IdleSpan::new(
        state,
        row.strict_instant("StartUtc")?,
        row.strict_instant("SettledThroughUtc")?,
    )
    .ok_or_else(|| row.invalid("SettledThroughUtc is before StartUtc"))?;
    Ok(StoredActivitySpan { id, span })
}

pub struct ActivitySourceSelectionRepository<'a> {
    connection: &'a Connection,
    clock: &'a dyn Clock,
}

impl<'a> ActivitySourceSelectionRepository<'a> {
    pub fn new(connection: &'a Connection, clock: &'a dyn Clock) -> Self {
        Self { connection, clock }
    }

    pub fn append(&self, selection: ActivitySourceSelection) -> Result<(), ActivityError> {
        self.insert(selection, false)?;
        Ok(())
    }

    /// Appends an ActivityWatch selection effective at the Unix epoch when no selection exists
    /// yet, for an installation set up before the built-in source. Returns false when a selection
    /// already existed.
    pub fn append_legacy_selection(&self) -> Result<bool, ActivityError> {
        self.insert(
            ActivitySourceSelection {
                effective_from: ActivitySourceSelection::LEGACY_EPOCH,
                source: ActivitySourceKind::ActivityWatch,
            },
            true,
        )
    }

    /// The selection in force at `start`, if any, followed by every selection that takes effect
    /// after `start` and before `end`, in effect order. Without `end`, every later selection.
    ///
    /// The .NET caller reads to `DateTimeOffset.MaxValue`, an instant past the last one a jiff
    /// `Timestamp` holds; `None` is that unbounded read.
    pub fn read_timeline(
        &self,
        start: Timestamp,
        end: Option<Timestamp>,
    ) -> Result<Vec<ActivitySourceSelection>, ActivityError> {
        if end.is_some_and(|end| end <= start) {
            return Err(ActivityError::EmptyInterval);
        }
        let mut statement = self.connection.prepare(
            "SELECT EffectiveFromUtc, Source FROM (
    SELECT EffectiveFromUtc, Source, Id
    FROM ActivitySourceSelection
    WHERE EffectiveFromUtc <= $start
    ORDER BY EffectiveFromUtc DESC, Id DESC
    LIMIT 1)
UNION ALL
SELECT EffectiveFromUtc, Source FROM (
    SELECT EffectiveFromUtc, Source, Id
    FROM ActivitySourceSelection
    WHERE EffectiveFromUtc > $start AND ($end IS NULL OR EffectiveFromUtc < $end)
    ORDER BY EffectiveFromUtc, Id);",
        )?;
        let rows = statement.query_map(
            named_params! {
                "$start": format_instant(start),
                "$end": end.map(format_instant),
            },
            |row| {
                Ok(parse_selection(&StoredRow::new(
                    row,
                    "ActivitySourceSelection",
                )))
            },
        )?;
        collect(rows)
    }

    fn insert(
        &self,
        selection: ActivitySourceSelection,
        only_if_empty: bool,
    ) -> Result<bool, ActivityError> {
        let condition = if only_if_empty {
            "WHERE NOT EXISTS (SELECT 1 FROM ActivitySourceSelection)"
        } else {
            ""
        };
        let inserted = self.connection.execute(
            &format!(
                "INSERT INTO ActivitySourceSelection (EffectiveFromUtc, Source, CreatedAtUtc)
SELECT $effective, $source, $created
{condition};"
            ),
            named_params! {
                "$effective": format_instant(selection.effective_from),
                "$source": source_storage(selection.source),
                "$created": format_instant(self.clock.now()),
            },
        )?;
        Ok(inserted == 1)
    }
}

fn parse_selection(row: &StoredRow) -> Result<ActivitySourceSelection, InvalidStoredValue> {
    let effective_from = row.strict_instant("EffectiveFromUtc")?;
    let source = match row.text("Source")? {
        "built-in" => ActivitySourceKind::BuiltIn,
        "activitywatch" => ActivitySourceKind::ActivityWatch,
        source => {
            return Err(row.invalid(format!(
                "stored activity source '{source}' is not recognized"
            )));
        }
    };
    Ok(ActivitySourceSelection {
        effective_from,
        source,
    })
}

#[cfg(test)]
mod tests;
