//! Idle rules of the built-in activity source, matching aw-watcher-afk on Windows.
//! See `specs/001-core-workday-indicator/contracts/built-in-activity-source.md`.

use jiff::{SignedDuration, Timestamp};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdleState {
    Active,
    Idle,
}

/// One `GetTickCount`/`GetLastInputInfo` pair, both in milliseconds of uptime modulo 2^32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastInputReading {
    pub tick_count: u32,
    pub last_input_tick: u32,
}

impl LastInputReading {
    pub fn idle_duration(self) -> SignedDuration {
        SignedDuration::from_millis(i64::from(
            self.tick_count.wrapping_sub(self.last_input_tick),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleSpan {
    state: IdleState,
    start: Timestamp,
    settled_through: Timestamp,
}

impl IdleSpan {
    /// `None` when the span would be settled before it starts.
    pub fn new(state: IdleState, start: Timestamp, settled_through: Timestamp) -> Option<Self> {
        (settled_through >= start).then_some(Self {
            state,
            start,
            settled_through,
        })
    }

    pub fn state(self) -> IdleState {
        self.state
    }

    pub fn start(self) -> Timestamp {
        self.start
    }

    pub fn settled_through(self) -> Timestamp {
        self.settled_through
    }

    fn settled_at(self, settled_through: Timestamp) -> Self {
        Self {
            settled_through,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleObservation {
    /// Same state as before; only the settled boundary of the open span may have advanced.
    Continued { open: IdleSpan },
    /// First sample of an uninterrupted observation. `abandoned` is the span that was open before
    /// the interruption, at its final settled boundary; nothing between it and `open` is known.
    Started {
        open: IdleSpan,
        abandoned: Option<IdleSpan>,
    },
    /// A transition seen within uninterrupted observation: `closed` ends where `open` starts.
    Transitioned { closed: IdleSpan, open: IdleSpan },
}

impl IdleObservation {
    pub fn open(self) -> IdleSpan {
        match self {
            Self::Continued { open }
            | Self::Started { open, .. }
            | Self::Transitioned { open, .. } => open,
        }
    }
}

pub struct IdleStateTracker {
    poll_interval: SignedDuration,
    open: Option<IdleSpan>,
    last_sample_at: Timestamp,
    last_input_tick: u32,
}

impl Default for IdleStateTracker {
    fn default() -> Self {
        Self::new(Self::DEFAULT_POLL_INTERVAL)
    }
}

impl IdleStateTracker {
    pub const DEFAULT_POLL_INTERVAL: SignedDuration = SignedDuration::from_secs(5);

    /// # Panics
    ///
    /// When `poll_interval` is not positive.
    pub fn new(poll_interval: SignedDuration) -> Self {
        assert!(
            poll_interval.is_positive(),
            "the poll interval must be positive"
        );
        Self {
            poll_interval,
            open: None,
            last_sample_at: Timestamp::UNIX_EPOCH,
            last_input_tick: 0,
        }
    }

    pub fn open(&self) -> Option<IdleSpan> {
        self.open
    }

    /// # Panics
    ///
    /// When `threshold` is not positive.
    pub fn observe(
        &mut self,
        at: Timestamp,
        reading: LastInputReading,
        threshold: SignedDuration,
    ) -> IdleObservation {
        assert!(
            threshold.is_positive(),
            "the idle threshold must be positive"
        );

        let idle_duration = reading.idle_duration();
        let last_input = at - idle_duration;
        let new_input = reading.last_input_tick != self.last_input_tick;
        let previous_sample_at = self.last_sample_at;
        self.last_input_tick = reading.last_input_tick;
        self.last_sample_at = at;

        let previous = match self.open {
            Some(previous)
                if at >= previous_sample_at
                    && at.duration_since(previous_sample_at) <= self.poll_interval * 2 =>
            {
                previous
            }
            abandoned => {
                let state = if idle_duration < threshold {
                    IdleState::Active
                } else {
                    IdleState::Idle
                };
                let open = IdleSpan {
                    state,
                    start: at,
                    settled_through: at,
                };
                self.open = Some(open);
                return IdleObservation::Started { open, abandoned };
            }
        };

        let observation = match previous.state {
            IdleState::Active if idle_duration >= threshold => {
                let idle_start = last_input.max(previous.settled_through);
                IdleObservation::Transitioned {
                    closed: previous.settled_at(idle_start),
                    open: IdleSpan {
                        state: IdleState::Idle,
                        start: idle_start,
                        settled_through: idle_start.max(at),
                    },
                }
            }
            IdleState::Active => IdleObservation::Continued {
                open: previous.settled_at(previous.settled_through.max(last_input.min(at))),
            },
            IdleState::Idle if new_input && idle_duration < threshold => {
                let active_start = last_input
                    .max(at - self.poll_interval)
                    .max(previous.settled_through);
                IdleObservation::Transitioned {
                    closed: previous.settled_at(active_start),
                    open: IdleSpan {
                        state: IdleState::Active,
                        start: active_start,
                        settled_through: active_start,
                    },
                }
            }
            IdleState::Idle => IdleObservation::Continued {
                open: previous.settled_at(at),
            },
        };
        self.open = Some(observation.open());
        observation
    }

    /// Ends uninterrupted observation (stop, suspend, resume, failed query, switch). Returns the
    /// span that was open, at its settled boundary, so it can be stored; the next sample starts a
    /// new span.
    pub fn interrupt(&mut self) -> Option<IdleSpan> {
        self.open.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: SignedDuration = SignedDuration::from_secs(180);

    fn nine() -> Timestamp {
        Timestamp::from_second(1_791_795_600).unwrap()
    }

    fn at(second: i64) -> Timestamp {
        nine() + SignedDuration::from_secs(second)
    }

    fn span(state: IdleState, start: i64, settled: i64) -> IdleSpan {
        IdleSpan::new(state, at(start), at(settled)).unwrap()
    }

    fn continued(state: IdleState, start: i64, settled: i64) -> IdleObservation {
        IdleObservation::Continued {
            open: span(state, start, settled),
        }
    }

    fn transitioned(closed: IdleSpan, open: IdleSpan) -> IdleObservation {
        IdleObservation::Transitioned { closed, open }
    }

    struct Machine {
        tick_at_nine: u32,
        last_input_tick: u32,
        tracker: IdleStateTracker,
        threshold: SignedDuration,
    }

    impl Machine {
        fn new() -> Self {
            Self::with_tick_at_nine(50_000_000)
        }

        fn with_tick_at_nine(tick_at_nine: u32) -> Self {
            Self {
                tick_at_nine,
                last_input_tick: 0,
                tracker: IdleStateTracker::default(),
                threshold: THRESHOLD,
            }
        }

        fn tick_at(&self, instant: Timestamp) -> u32 {
            let millis = instant.duration_since(nine()).as_millis() as i32;
            self.tick_at_nine.wrapping_add(millis as u32)
        }

        fn input(&mut self, instant: Timestamp) {
            self.last_input_tick = self.tick_at(instant);
        }

        fn sample(&mut self, instant: Timestamp) -> IdleObservation {
            let reading = LastInputReading {
                tick_count: self.tick_at(instant),
                last_input_tick: self.last_input_tick,
            };
            self.tracker.observe(instant, reading, self.threshold)
        }

        fn sample_every_5_seconds(&mut self, from: i64, through: i64) {
            for second in (from..=through).step_by(5) {
                self.sample(at(second));
            }
        }

        fn idle_since(last_input: i64, sampled_through: i64) -> Self {
            let mut machine = Self::new();
            machine.input(at(last_input));
            machine.sample(at(last_input));
            machine.sample_every_5_seconds(last_input + 5, sampled_through);
            assert_eq!(machine.tracker.open().unwrap().state(), IdleState::Idle);
            machine
        }
    }

    #[test]
    fn idle_duration_uses_unsigned_tick_arithmetic_across_wraparound() {
        let reading = LastInputReading {
            tick_count: 1_000,
            last_input_tick: u32::MAX - 1_999,
        };

        assert_eq!(reading.idle_duration(), SignedDuration::from_millis(3_000));
    }

    #[test]
    fn crossing_the_threshold_backdates_idle_to_the_last_input() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample(at(0));
        machine.sample_every_5_seconds(5, 170);

        let still_active = machine.sample(at(175));
        let crossed = machine.sample(at(180));

        assert_eq!(still_active, continued(IdleState::Active, 0, 0));
        assert_eq!(
            crossed,
            transitioned(span(IdleState::Active, 0, 0), span(IdleState::Idle, 0, 180))
        );
    }

    #[test]
    fn exact_threshold_is_idle_and_one_millisecond_less_is_active() {
        let mut tracker = IdleStateTracker::default();
        for elapsed in (0..180_000u32).step_by(5_000) {
            tracker.observe(
                at(0) + SignedDuration::from_millis(i64::from(elapsed)),
                LastInputReading {
                    tick_count: 10_000 + elapsed,
                    last_input_tick: 10_000,
                },
                THRESHOLD,
            );
        }

        let below = tracker.observe(
            at(0) + THRESHOLD - SignedDuration::from_millis(1),
            LastInputReading {
                tick_count: 189_999,
                last_input_tick: 10_000,
            },
            THRESHOLD,
        );
        let exact = tracker.observe(
            at(0) + THRESHOLD,
            LastInputReading {
                tick_count: 190_000,
                last_input_tick: 10_000,
            },
            THRESHOLD,
        );

        assert!(matches!(below, IdleObservation::Continued { .. }));
        assert!(
            matches!(exact, IdleObservation::Transitioned { open, .. } if open.state() == IdleState::Idle)
        );
    }

    #[test]
    fn threshold_crossing_across_tick_wraparound_backdates_to_the_last_input() {
        let mut machine = Machine::with_tick_at_nine(u32::MAX - 60_000);
        machine.input(at(0));
        machine.sample(at(0));
        for second in (5..180).step_by(5) {
            assert!(matches!(
                machine.sample(at(second)),
                IdleObservation::Continued { .. }
            ));
        }

        let crossed = machine.sample(at(180));

        assert_eq!(
            crossed,
            transitioned(span(IdleState::Active, 0, 0), span(IdleState::Idle, 0, 180))
        );
    }

    #[test]
    fn active_span_settles_only_through_the_last_input_a_sample_read() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample_every_5_seconds(0, 40);
        machine.input(at(42));
        machine.sample(at(45));

        machine.sample(at(50));
        machine.sample(at(55));

        assert_eq!(machine.tracker.open(), Some(span(IdleState::Active, 0, 42)));
    }

    #[test]
    fn new_input_while_idle_returns_to_active_from_that_input() {
        let mut machine = Machine::idle_since(0, 600);
        machine.input(at(602));

        let returned = machine.sample(at(605));

        assert_eq!(
            returned,
            transitioned(
                span(IdleState::Idle, 0, 602),
                span(IdleState::Active, 602, 602)
            )
        );
    }

    #[test]
    fn return_to_active_is_backdated_at_most_one_polling_interval() {
        let mut machine = Machine::idle_since(0, 600);
        machine.input(at(601));

        let returned = machine.sample(at(608));

        assert_eq!(
            returned,
            transitioned(
                span(IdleState::Idle, 0, 603),
                span(IdleState::Active, 603, 603)
            )
        );
    }

    #[test]
    fn start_while_already_idle_is_not_backdated_before_start() {
        let mut machine = Machine::new();
        machine.input(at(-600));

        let started = machine.sample(at(0));
        let next = machine.sample(at(5));

        assert_eq!(
            started,
            IdleObservation::Started {
                open: span(IdleState::Idle, 0, 0),
                abandoned: None
            }
        );
        assert_eq!(next, continued(IdleState::Idle, 0, 5));
    }

    #[test]
    fn idle_detected_after_a_start_while_inactive_is_not_backdated_before_start() {
        let mut machine = Machine::new();
        machine.input(at(-60));
        machine.sample(at(0));
        for second in (5..120).step_by(5) {
            machine.sample(at(second));
        }

        let crossed = machine.sample(at(120));

        assert_eq!(
            crossed,
            transitioned(span(IdleState::Active, 0, 0), span(IdleState::Idle, 0, 120))
        );
    }

    #[test]
    fn suspend_mid_activity_ends_the_span_at_its_last_input_and_resume_starts_a_new_one() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample_every_5_seconds(0, 25);
        machine.input(at(28));
        machine.sample(at(30));
        machine.sample(at(35));

        let suspended = machine.tracker.interrupt();
        machine.input(at(3_600));
        let resumed = machine.sample(at(3_601));

        assert_eq!(suspended, Some(span(IdleState::Active, 0, 28)));
        assert_eq!(
            resumed,
            IdleObservation::Started {
                open: span(IdleState::Active, 3_601, 3_601),
                abandoned: None
            }
        );
    }

    #[test]
    fn locking_does_not_change_the_idle_rules() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample(at(0));

        // The session locks at 10 s: the screen accepts no input but sampling goes on.
        machine.sample_every_5_seconds(5, 175);
        let idle_while_locked = machine.sample(at(180));
        machine.sample_every_5_seconds(185, 300);
        machine.input(at(302));
        let unlocked = machine.sample(at(305));

        assert_eq!(idle_while_locked.open(), span(IdleState::Idle, 0, 180));
        assert_eq!(
            unlocked,
            transitioned(
                span(IdleState::Idle, 0, 302),
                span(IdleState::Active, 302, 302)
            )
        );
    }

    #[test]
    fn missed_samples_then_recovery_in_the_same_state_appends_a_new_span_without_bridging() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample(at(0));
        machine.input(at(9));
        machine.sample(at(10));
        machine.input(at(20));

        let recovered = machine.sample(at(21));

        assert_eq!(
            recovered,
            IdleObservation::Started {
                open: span(IdleState::Active, 21, 21),
                abandoned: Some(span(IdleState::Active, 0, 9))
            }
        );
    }

    #[test]
    fn samples_exactly_two_polling_intervals_apart_stay_uninterrupted() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample(at(0));
        machine.input(at(9));

        assert!(matches!(
            machine.sample(at(10)),
            IdleObservation::Continued { .. }
        ));
    }

    #[test]
    fn a_clock_that_moves_backward_interrupts_observation() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample(at(0));
        machine.sample(at(5));

        assert!(matches!(
            machine.sample(at(2)),
            IdleObservation::Started { .. }
        ));
    }

    #[test]
    fn a_failed_query_interrupts_and_the_next_successful_sample_starts_fresh() {
        let mut machine = Machine::idle_since(0, 300);

        let ended = machine.tracker.interrupt();
        let recovered = machine.sample(at(305));

        assert_eq!(ended, Some(span(IdleState::Idle, 0, 300)));
        assert_eq!(
            recovered,
            IdleObservation::Started {
                open: span(IdleState::Idle, 305, 305),
                abandoned: None
            }
        );
    }

    #[test]
    fn lowering_the_threshold_mid_day_settles_the_pending_tail_as_idle_from_the_last_input() {
        let mut machine = Machine::new();
        machine.input(at(0));
        machine.sample(at(0));
        machine.sample_every_5_seconds(5, 85);

        machine.threshold = SignedDuration::from_secs(60);
        let crossed = machine.sample(at(90));

        assert_eq!(
            crossed,
            transitioned(span(IdleState::Active, 0, 0), span(IdleState::Idle, 0, 90))
        );
    }

    #[test]
    fn raising_the_threshold_while_idle_without_new_input_stays_idle() {
        let mut machine = Machine::idle_since(0, 200);

        machine.threshold = SignedDuration::from_mins(30);
        let next = machine.sample(at(205));

        assert_eq!(next, continued(IdleState::Idle, 0, 205));
    }
}
