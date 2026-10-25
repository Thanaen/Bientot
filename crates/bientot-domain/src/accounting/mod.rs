//! Accounting of the workday from activity evidence and recorded transitions.

pub mod engine;
pub mod evidence;
pub mod model;

pub use engine::calculate;
pub use evidence::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AppointmentWindow, AutomaticOrigin, BlankSourceId, InvalidAccountingInput,
    NonPositiveReviewThreshold, OperationKey, PowerState, Recorded, RecordedTransition,
    SessionState, TrackingState, WorkdayEventHistory, WorkdayEventId, WorkdayStateAnchors,
    WorkdayTransition, WorkdayTransitionKind,
};
pub use model::{
    AccountedOrigin, AccountedSegment, AccountingLabel, AccountingResult, AccountingTimeline,
    AdjustmentId, BlankIdentifier, LongLockReview, LongLockReviewId, LongLockReviewReason,
    ManualDurationCredit, SegmentKind,
};
