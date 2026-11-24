use std::cell::Cell;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::*;

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn message(message: u32, w_param: usize, l_param: isize) -> WindowMessage {
    WindowMessage {
        message,
        w_param,
        l_param,
    }
}

#[test]
fn session_changes_decode_only_the_lock_and_unlock_of_this_session() {
    let decoded: Vec<_> = [
        message(0x02B1, 0x7, 73),
        message(0x02B1, 0x8, 73),
        message(0x000F, 0x7, 73),
        message(0x02B1, 0x5, 73),
        message(0x02B1, 0x7, 74),
        message(0x02B1, 0x7, 73 + (1 << 32)),
    ]
    .into_iter()
    .map(|message| decode(message, 73))
    .collect();

    assert_eq!(
        decoded,
        vec![
            Some(LifecycleChange::Session(SessionState::Locked)),
            Some(LifecycleChange::Session(SessionState::Unlocked)),
            None,
            None,
            None,
            Some(LifecycleChange::Session(SessionState::Locked)),
        ]
    );
}

#[test]
fn power_broadcasts_decode_only_suspend_and_one_resume() {
    let decoded: Vec<_> = [
        message(0x0218, 0x4, 0),
        message(0x0218, 0x12, 0),
        message(0x0218, 0x7, 0),
        message(0x0218, 0x6, 0),
        message(0x000F, 0x4, 0),
    ]
    .into_iter()
    .map(|message| decode(message, 73))
    .collect();

    assert_eq!(
        decoded,
        vec![
            Some(LifecycleChange::Power(PowerState::Suspended)),
            Some(LifecycleChange::Power(PowerState::Active)),
            None,
            None,
            None,
        ]
    );
}

#[test]
fn session_flags_map_lock_and_unlock_and_anything_else_or_a_failed_query_to_unknown() {
    assert_eq!(
        [Some(0), Some(1), Some(2), None].map(session_state),
        [
            SessionState::Locked,
            SessionState::Unlocked,
            SessionState::Unknown,
            SessionState::Unknown,
        ]
    );
}

struct CountingClock {
    now: Timestamp,
    reads: Cell<usize>,
}

impl Clock for CountingClock {
    fn now(&self) -> Timestamp {
        self.reads.set(self.reads.get() + 1);
        self.now
    }
}

struct CountingZones {
    reads: Cell<usize>,
}

impl TimeZoneProvider for CountingZones {
    fn local(&self) -> (String, TimeZone) {
        self.reads.set(self.reads.get() + 1);
        ("Europe/Paris".into(), TimeZone::fixed(jiff::tz::offset(2)))
    }

    fn get(&self, id: &str) -> Result<TimeZone, bientot_domain::time::UnknownTimeZone> {
        Err(bientot_domain::time::UnknownTimeZone(id.into()))
    }
}

#[test]
fn each_change_reads_the_clock_and_the_local_zone_once_and_an_ignored_message_reads_neither() {
    let captured_at = at("2026-08-30T09:42:17Z");
    let messages = LifecycleMessages::new(
        73,
        CountingClock {
            now: captured_at,
            reads: Cell::new(0),
        },
        CountingZones {
            reads: Cell::new(0),
        },
    );

    let transitions = [
        message(0x02B1, 0x7, 73),
        message(0x0218, 0x7, 0),
        message(0x0218, 0x4, 0),
    ]
    .map(|message| messages.transition(message));

    let paris = |change| {
        Some(LifecycleTransition {
            change,
            occurred_at: captured_at,
            time_zone_id: "Europe/Paris".into(),
        })
    };
    assert_eq!(
        transitions,
        [
            paris(LifecycleChange::Session(SessionState::Locked)),
            None,
            paris(LifecycleChange::Power(PowerState::Suspended)),
        ]
    );
    assert_eq!(
        (messages.clock.reads.get(), messages.zones.reads.get()),
        (2, 2)
    );
}
