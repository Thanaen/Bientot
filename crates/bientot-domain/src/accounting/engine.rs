//! Classification of the workday into credited, provisional, excluded and unknown time.

use std::collections::{BTreeSet, HashSet};

use jiff::{SignedDuration, Timestamp};

use super::evidence::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivityState, AutomaticOrigin,
    COVERAGE_GAP_ORIGIN, PowerState, Recorded, RecordedTransition, SESSION_HISTORY_SOURCE_ID,
    SessionState, TrackingState, WorkdayEventId, WorkdayStateAnchors,
};
use super::model::{
    AccountedOrigin, AccountedSegment, AccountingLabel, AccountingResult, AccountingTimeline,
    LongLockReview, LongLockReviewId, LongLockReviewReason, SegmentKind, append_canonical,
};
use crate::adjustments::{self, AdjustmentValidationError};
use crate::intervals::UtcInterval;
use crate::time::WorkdayWindow;

const MISSING_ACTIVITY_ORIGIN: &str = "activity:missing";

/// How far before or after its hours an absence still belongs to an appointment.
const APPOINTMENT_TOLERANCE: SignedDuration = SignedDuration::from_mins(15);

/// Shorter absences during an appointment are left as they are.
const MINIMUM_APPOINTMENT_ABSENCE: SignedDuration = SignedDuration::from_mins(5);

pub fn calculate(input: &AccountingInput) -> Result<AccountingResult, AdjustmentValidationError> {
    let window = input.window();
    let through = input.now().clamp(window.start, window.end);
    if through == window.start {
        let empty = adjustments::apply(
            AccountingTimeline::new(window.clone(), through, Vec::new()),
            input.adjustments(),
        )?;
        return Ok(AccountingResult::new(
            empty.timeline,
            empty.duration_credits,
            Vec::new(),
        ));
    }

    let adjusted = adjustments::active_intervals(input.adjustments());
    let cut_points = cut_points(input, &adjusted, through);
    let lock_episodes = lock_episodes(input, through);
    let overnight_locks: HashSet<_> = lock_episodes
        .iter()
        .filter(|episode| ends_workday(episode, window, through))
        .map(|episode| &episode.id)
        .collect();
    let lunch_locks: HashSet<_> = lock_episodes
        .iter()
        .filter(|episode| {
            !overnight_locks.contains(&episode.id) && joins_lunch(episode, input.policy())
        })
        .map(|episode| &episode.id)
        .collect();

    let mut transitions = input
        .events()
        .in_window()
        .iter()
        .filter(|transition| transition.occurred_at() <= through)
        .peekable();
    let mut state = AccountingState::from(input.events().before_window());
    let mut pieces = Vec::with_capacity(cut_points.len() - 1);
    for pair in cut_points.windows(2) {
        let start = pair[0];
        while let Some(transition) = transitions.next_if(|next| next.occurred_at() <= start) {
            state.apply(transition);
        }
        let interval = UtcInterval::new(start, pair[1]).expect("cut points are strictly sorted");
        let segment = classify(interval, input, &state, &overnight_locks, &lunch_locks);
        // Sleep while locked stays the lock's to review; lunch is never work; what the user
        // already said is not asked again.
        let away = matches!(
            segment.label(),
            AccountingLabel::Afk | AccountingLabel::Suspended
        ) && state.tracking_state() == TrackingState::Running
            && state.session_state() != SessionState::Locked
            && !input.policy().lunch().contains(interval.start())
            && !adjusted
                .iter()
                .any(|adjusted| adjusted.contains(interval.start()));
        pieces.push(Piece { segment, away });
    }
    let appointment_episodes = cover_appointments(&mut pieces, input.policy(), window, through);
    let rebuilt_episodes = rebuilt_episodes(&mut pieces, window, through);
    let mut segments = Vec::with_capacity(pieces.len());
    for piece in pieces {
        append_canonical(&mut segments, piece.segment);
    }

    let automatic = AccountingTimeline::new(window.clone(), through, segments);
    let adjusted = adjustments::apply(automatic, input.adjustments())?;
    let pending_reviews = pending_reviews(
        input.policy(),
        &lock_episodes,
        &appointment_episodes,
        &rebuilt_episodes,
        &adjusted.timeline,
    );
    Ok(AccountingResult::new(
        adjusted.timeline,
        adjusted.duration_credits,
        pending_reviews,
    ))
}

/// `adjusted` cuts at its edges so that an appointment's absence stops where an answer begins.
fn cut_points(
    input: &AccountingInput,
    adjusted: &[UtcInterval],
    through: Timestamp,
) -> Vec<Timestamp> {
    let start = input.window().start;
    let lunch = input.policy().lunch();
    let candidates = [lunch.start(), lunch.end()]
        .into_iter()
        .chain(
            input
                .policy()
                .appointments()
                .iter()
                .flat_map(|appointment| {
                    let hours = appointment.interval;
                    let reach = reach(hours);
                    [hours.start(), hours.end(), reach.start(), reach.end()]
                }),
        )
        .chain(
            input
                .activity()
                .segments()
                .iter()
                .flat_map(|segment| [segment.interval.start(), segment.interval.end()]),
        )
        .chain(
            input
                .events()
                .in_window()
                .iter()
                .map(RecordedTransition::occurred_at),
        )
        .chain(
            adjusted
                .iter()
                .flat_map(|interval| [interval.start(), interval.end()]),
        )
        .filter(|&candidate| start < candidate && candidate < through);
    let points: BTreeSet<_> = [start, through].into_iter().chain(candidates).collect();
    points.into_iter().collect()
}

/// A lock that is not lifted before its workday ends marks the end of that day (D-003).
fn ends_workday(episode: &LockEpisode, window: &WorkdayWindow, through: Timestamp) -> bool {
    episode.interval.start() < window.start || (episode.is_open && through == window.end)
}

/// A lock that crosses one lunch boundary and stays outside lunch for less than the review
/// threshold is a lunch taken a little early or late (issue #136), not work to confirm.
fn joins_lunch(episode: &LockEpisode, policy: &AccountingPolicy) -> bool {
    let (locked, lunch) = (episode.interval, policy.lunch());
    let starts_before = locked.start() < lunch.start();
    let ends_after = locked.end() > lunch.end();
    if !locked.overlaps(lunch) || starts_before == ends_after {
        return false;
    }
    let outside = if starts_before {
        lunch.start().duration_since(locked.start())
    } else {
        locked.end().duration_since(lunch.end())
    };
    outside < policy.long_lock_review_threshold()
}

/// A classified piece between two cut points, and whether it counts as away for an
/// appointment: idle or asleep on a running day, neither locked nor at lunch.
struct Piece {
    segment: AccountedSegment,
    away: bool,
}

fn reach(hours: UtcInterval) -> UtcInterval {
    UtcInterval::new(
        hours.start() - APPOINTMENT_TOLERANCE,
        hours.end() + APPOINTMENT_TOLERANCE,
    )
    .expect("widening keeps a positive duration")
}

/// Credits provisionally each absence during an appointment, under a review of its own (issue
/// #321): every run of away pieces within the appointment's reach that overlaps its hours and
/// lasts [`MINIMUM_APPOINTMENT_ABSENCE`], once the user has worked earlier that day. A review
/// per run keeps the work between two absences out of their answers.
fn cover_appointments(
    pieces: &mut [Piece],
    policy: &AccountingPolicy,
    window: &WorkdayWindow,
    through: Timestamp,
) -> Vec<CoveredEpisode> {
    let mut covered = Vec::new();
    for appointment in policy.appointments() {
        let (hours, reach) = (appointment.interval, reach(appointment.interval));
        let within = |piece: &Piece| {
            let interval = piece.segment.interval();
            piece.away && reach.start() <= interval.start() && interval.end() <= reach.end()
        };
        let mut runs = Vec::new();
        let mut start = 0;
        for chunk in pieces.chunk_by(|a, b| within(a) == within(b)) {
            let range = start..start + chunk.len();
            start = range.end;
            if !within(&chunk[0]) {
                continue;
            }
            let run = UtcInterval::new(
                chunk[0].segment.interval().start(),
                chunk[chunk.len() - 1].segment.interval().end(),
            )
            .expect("pieces have a positive duration");
            if run.overlaps(hours) && run.duration() >= MINIMUM_APPOINTMENT_ABSENCE {
                runs.push((range, run));
            }
        }
        let Some((first, _)) = runs.first() else {
            continue;
        };
        // Only confirmed work shows the user was there: not a lock nor another absence.
        let worked_before = pieces[..first.start]
            .iter()
            .any(|piece| *piece.segment.kind() == SegmentKind::Credited);
        if !worked_before {
            continue;
        }
        for (range, run) in runs {
            let review_id =
                LongLockReviewId::for_appointment(&appointment.appointment_id, run.start());
            for piece in &mut pieces[range] {
                piece.segment = AccountedSegment::provisional_credit(
                    piece.segment.interval(),
                    AccountingLabel::ScheduledAway,
                    piece.segment.origin().clone(),
                    review_id.clone(),
                );
                piece.away = false;
            }
            covered.push(CoveredEpisode {
                id: review_id,
                interval: run,
                is_open: run.end() == through && through < window.end,
            });
        }
    }
    covered
}

/// One review over the provisional pieces it covers: an absence during an appointment, or the
/// time a session was open before Bientôt watched it.
struct CoveredEpisode {
    id: LongLockReviewId,
    interval: UtcInterval,
    is_open: bool,
}

/// One review per run of rebuilt pieces, so that an answer never covers the lunch or the sleep
/// between two runs of one unlock: each run takes the review of its start.
fn rebuilt_episodes(
    pieces: &mut [Piece],
    window: &WorkdayWindow,
    through: Timestamp,
) -> Vec<CoveredEpisode> {
    let rebuilt = |piece: &Piece| piece.segment.label() == AccountingLabel::RebuiltWork;
    let mut episodes = Vec::new();
    for run in pieces.chunk_by_mut(|a, b| rebuilt(a) == rebuilt(b)) {
        if !rebuilt(&run[0]) {
            continue;
        }
        let interval = UtcInterval::new(
            run[0].segment.interval().start(),
            run[run.len() - 1].segment.interval().end(),
        )
        .expect("pieces have a positive duration");
        let SegmentKind::ProvisionalCredit { review_id: id } = run[0].segment.kind().clone() else {
            unreachable!("rebuilt time awaits a review");
        };
        for piece in run.iter_mut() {
            piece.segment = AccountedSegment::provisional_credit(
                piece.segment.interval(),
                AccountingLabel::RebuiltWork,
                piece.segment.origin().clone(),
                id.clone(),
            );
        }
        episodes.push(CoveredEpisode {
            id,
            interval,
            is_open: interval.end() == through && through < window.end,
        });
    }
    episodes
}

fn classify(
    interval: UtcInterval,
    input: &AccountingInput,
    state: &AccountingState<'_>,
    overnight_locks: &HashSet<&LongLockReviewId>,
    lunch_locks: &HashSet<&LongLockReviewId>,
) -> AccountedSegment {
    if let Some(power) = state
        .power
        .filter(|power| power.state == PowerState::Suspended)
    {
        return AccountedSegment::excluded(
            interval,
            AccountingLabel::Suspended,
            power.origin.clone(),
        );
    }
    if let Some(tracking) = state.tracking {
        let label = match tracking.state {
            TrackingState::Running => None,
            TrackingState::Paused => Some(AccountingLabel::ManualPause),
            TrackingState::Finished => Some(AccountingLabel::DayFinished),
        };
        if let Some(label) = label {
            return AccountedSegment::excluded(interval, label, tracking.origin.clone());
        }
    }
    if let Some(session) = state
        .session
        .filter(|session| session.state == SessionState::Locked)
    {
        let origin = session.origin.clone();
        if input.policy().lunch().contains(interval.start()) {
            return AccountedSegment::excluded(interval, AccountingLabel::LunchLock, origin);
        }
        let review_id = review_id(session.id);
        if overnight_locks.contains(&review_id) {
            return AccountedSegment::excluded(interval, AccountingLabel::OvernightLock, origin);
        }
        if lunch_locks.contains(&review_id) {
            return AccountedSegment::excluded(interval, AccountingLabel::LunchLock, origin);
        }
        return AccountedSegment::provisional_credit(
            interval,
            AccountingLabel::LockedWork,
            origin,
            review_id,
        );
    }
    let segment = classify_activity(interval, input.activity());
    match state.session {
        Some(session) if rebuilds(session, &segment, input.policy(), interval) => {
            AccountedSegment::provisional_credit(
                interval,
                AccountingLabel::RebuiltWork,
                session.origin.clone(),
                LongLockReviewId::for_rebuilt_session(session.id, interval.start()),
            )
        }
        _ => segment,
    }
}

/// Time without activity evidence while a session the Windows logs say was unlocked, before
/// Bientôt watched it, counts provisionally (issue #75); lunch stays unknown, as nothing says it
/// was work.
fn rebuilds(
    session: &Recorded<SessionState>,
    segment: &AccountedSegment,
    policy: &AccountingPolicy,
    interval: UtcInterval,
) -> bool {
    session.state == SessionState::Unlocked
        && session.origin.source_id() == SESSION_HISTORY_SOURCE_ID
        && has_no_evidence(segment)
        && !policy.lunch().contains(interval.start())
}

/// No source covered the segment, or the one in use left it uncovered: before Bientôt ran.
fn has_no_evidence(segment: &AccountedSegment) -> bool {
    matches!(
        segment.origin(),
        AccountedOrigin::Automatic(origin)
            if [MISSING_ACTIVITY_ORIGIN, COVERAGE_GAP_ORIGIN].contains(&origin.source_id())
    )
}

fn missing_activity_origin() -> AutomaticOrigin {
    AutomaticOrigin::new(MISSING_ACTIVITY_ORIGIN).expect("the origin is not blank")
}

fn classify_activity(interval: UtcInterval, coverage: &ActivityCoverage) -> AccountedSegment {
    let evidence: Vec<_> = coverage
        .segments()
        .iter()
        .filter(|segment| segment.interval.contains(interval.start()))
        .collect();
    let Some(first) = evidence.first() else {
        return AccountedSegment::unknown(interval, missing_activity_origin());
    };

    let source_ids: BTreeSet<_> = evidence
        .iter()
        .map(|segment| segment.origin.source_id())
        .collect();
    let origin = AutomaticOrigin::new(source_ids.into_iter().collect::<Vec<_>>().join("+"))
        .expect("source identifiers are not blank");
    if evidence.iter().any(|segment| segment.state != first.state) {
        return AccountedSegment::unknown(interval, origin);
    }
    match first.state {
        ActivityState::Active => {
            AccountedSegment::credited(interval, AccountingLabel::ActiveWork, origin)
        }
        ActivityState::Afk => AccountedSegment::excluded(interval, AccountingLabel::Afk, origin),
        ActivityState::Unknown => AccountedSegment::unknown(interval, origin),
    }
}

fn pending_reviews(
    policy: &AccountingPolicy,
    lock_episodes: &[LockEpisode],
    appointment_episodes: &[CoveredEpisode],
    rebuilt_episodes: &[CoveredEpisode],
    timeline: &AccountingTimeline,
) -> Vec<LongLockReview> {
    let provisional = timeline.provisional_coverage_by_review();
    let review = |id: &LongLockReviewId, interval, reason, is_open| {
        Some(LongLockReview {
            id: id.clone(),
            locked_episode: interval,
            provisional_coverage: provisional.get(id)?.clone(),
            reason,
            is_open,
        })
    };
    let locks = lock_episodes
        .iter()
        .filter(|episode| episode.interval.duration() >= policy.long_lock_review_threshold())
        .filter_map(|episode| {
            review(
                &episode.id,
                episode.interval,
                LongLockReviewReason::ThresholdExceeded,
                episode.is_open,
            )
        });
    let appointments = appointment_episodes.iter().filter_map(|episode| {
        review(
            &episode.id,
            episode.interval,
            LongLockReviewReason::RecurringAppointment,
            episode.is_open,
        )
    });
    let rebuilt = rebuilt_episodes.iter().filter_map(|episode| {
        review(
            &episode.id,
            episode.interval,
            LongLockReviewReason::RebuiltSession,
            episode.is_open,
        )
    });
    locks.chain(appointments).chain(rebuilt).collect()
}

struct LockEpisode {
    id: LongLockReviewId,
    interval: UtcInterval,
    is_open: bool,
}

fn lock_episodes(input: &AccountingInput, through: Timestamp) -> Vec<LockEpisode> {
    let mut current = input
        .events()
        .before_window()
        .session
        .as_ref()
        .filter(|anchor| anchor.state == SessionState::Locked)
        .map(|anchor| (anchor.occurred_at, review_id(anchor.id)));
    let mut episodes = Vec::new();
    let sessions = input
        .events()
        .in_window()
        .iter()
        .filter_map(|transition| match transition {
            RecordedTransition::Session(session) if session.occurred_at <= through => Some(session),
            _ => None,
        });
    for session in sessions {
        match (session.state, current.take()) {
            (SessionState::Locked, None) => {
                current = Some((session.occurred_at, review_id(session.id)));
            }
            (SessionState::Locked, open) => current = open,
            (_, Some((start, id))) => {
                if let Ok(interval) = UtcInterval::new(start, session.occurred_at) {
                    episodes.push(LockEpisode {
                        id,
                        interval,
                        is_open: false,
                    });
                }
            }
            (_, None) => {}
        }
    }
    if let Some((start, id)) = current
        && let Ok(interval) = UtcInterval::new(start, through)
    {
        episodes.push(LockEpisode {
            id,
            interval,
            is_open: true,
        });
    }
    episodes
}

fn review_id(event_id: WorkdayEventId) -> LongLockReviewId {
    LongLockReviewId::new(format!("session-lock:{}", event_id.0)).expect("the id is not blank")
}

/// The transition that set each lane's current state; `None` while a lane keeps its default
/// (unknown session, active power, running tracking).
struct AccountingState<'a> {
    session: Option<&'a Recorded<SessionState>>,
    power: Option<&'a Recorded<PowerState>>,
    tracking: Option<&'a Recorded<TrackingState>>,
}

impl<'a> AccountingState<'a> {
    fn from(anchors: &'a WorkdayStateAnchors) -> Self {
        Self {
            session: anchors.session.as_ref(),
            power: anchors.power.as_ref(),
            tracking: anchors.tracking.as_ref(),
        }
    }

    fn apply(&mut self, transition: &'a RecordedTransition) {
        match transition {
            RecordedTransition::Session(session) => {
                let still_locked = session.state == SessionState::Locked
                    && self.session_state() == SessionState::Locked;
                if !still_locked {
                    self.session = Some(session);
                }
            }
            RecordedTransition::Power(power) => {
                if power.state != self.power_state() {
                    self.power = Some(power);
                }
            }
            RecordedTransition::Tracking(tracking) => {
                if tracking.state != self.tracking_state() {
                    self.tracking = Some(tracking);
                }
            }
        }
    }

    fn session_state(&self) -> SessionState {
        self.session
            .map_or(SessionState::Unknown, |session| session.state)
    }

    fn power_state(&self) -> PowerState {
        self.power.map_or(PowerState::Active, |power| power.state)
    }

    fn tracking_state(&self) -> TrackingState {
        self.tracking
            .map_or(TrackingState::Running, |tracking| tracking.state)
    }
}

#[cfg(test)]
mod tests;
