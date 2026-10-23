//! Activity sources: their health, coverage gaps and the idle rules of the built-in source.

pub mod diagnostics;
pub mod gaps;
pub mod health;
pub mod idle;

use jiff::Timestamp;

pub use diagnostics::{
    InvalidDiagnostics, SourceDiagnosticInterval, SourceDiagnostics, SourceIntervalKind,
};
pub use gaps::complete_unknown_gaps;
pub use health::{NotDegraded, SourceFreshness, SourceHealth, SourceQuality};
pub use idle::{IdleObservation, IdleSpan, IdleState, IdleStateTracker, LastInputReading};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActivitySourceKind {
    BuiltIn,
    ActivityWatch,
}

/// The activity source that answers for every instant from `effective_from` until the next selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivitySourceSelection {
    pub effective_from: Timestamp,
    pub source: ActivitySourceKind,
}

impl ActivitySourceSelection {
    pub const LEGACY_EPOCH: Timestamp = Timestamp::UNIX_EPOCH;
}
