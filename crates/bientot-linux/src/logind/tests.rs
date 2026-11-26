use super::*;

const LOCKED: Option<Step> = Some(Step::Changed(LifecycleChange::Session(
    SessionState::Locked,
)));
const UNLOCKED: Option<Step> = Some(Step::Changed(LifecycleChange::Session(
    SessionState::Unlocked,
)));
const SUSPENDED: Option<Step> = Some(Step::Changed(LifecycleChange::Power(PowerState::Suspended)));
const RESUMED: Option<Step> = Some(Step::Changed(LifecycleChange::Power(PowerState::Active)));

fn tracker(locked_hint: bool, active: bool, screensaver: Option<bool>) -> SessionTracker {
    SessionTracker::new(SessionReadings {
        locked_hint,
        active,
        screensaver,
    })
}

/// A session where logind reports the lock: the desktop has set `LockedHint` once already.
fn reporting() -> SessionTracker {
    let mut tracker = tracker(true, true, None);
    tracker.apply(locked_hint(false));
    tracker
}

fn locked_hint(locked: bool) -> Signal {
    Signal::SessionChanged {
        locked_hint: Some(locked),
        active: None,
    }
}

fn active(active: bool) -> Signal {
    Signal::SessionChanged {
        locked_hint: None,
        active: Some(active),
    }
}

fn apply_all(tracker: &mut SessionTracker, signals: &[Signal]) -> Vec<Option<Step>> {
    signals
        .iter()
        .map(|&signal| tracker.apply(signal))
        .collect()
}

fn snapshot(session: SessionState, lock_reporting: LockReporting) -> SessionSnapshot {
    SessionSnapshot {
        session,
        lock_reporting,
    }
}

#[test]
fn locked_hint_locks_and_unlocks_the_session() {
    assert_eq!(
        apply_all(&mut reporting(), &[locked_hint(true), locked_hint(false)]),
        [LOCKED, UNLOCKED]
    );
}

#[test]
fn a_switched_away_session_is_locked_and_unlocks_on_return_unless_its_screen_is_locked() {
    assert_eq!(
        apply_all(
            &mut reporting(),
            &[
                active(false),
                active(true),
                locked_hint(true),
                active(false),
                active(true),
                locked_hint(false),
            ]
        ),
        [LOCKED, UNLOCKED, LOCKED, None, None, UNLOCKED]
    );
}

#[test]
fn both_properties_in_one_change_record_one_session_change() {
    let both = |locked, is_active| Signal::SessionChanged {
        locked_hint: Some(locked),
        active: Some(is_active),
    };

    assert_eq!(
        apply_all(
            &mut reporting(),
            &[both(true, false), both(false, false), both(false, true)]
        ),
        [LOCKED, None, UNLOCKED]
    );
}

#[test]
fn prepare_for_sleep_suspends_and_resumes_once_each() {
    assert_eq!(
        apply_all(
            &mut reporting(),
            &[
                Signal::PrepareForSleep(true),
                Signal::PrepareForSleep(true),
                Signal::PrepareForSleep(false),
                Signal::PrepareForSleep(false),
            ]
        ),
        [SUSPENDED, None, RESUMED, None]
    );
}

#[test]
fn a_repeated_signal_records_nothing() {
    let mut tracker = reporting();

    assert_eq!(
        apply_all(
            &mut tracker,
            &[
                locked_hint(false),
                active(true),
                locked_hint(true),
                locked_hint(true),
                Signal::ScreenSaverActive(true),
            ]
        ),
        [None, None, LOCKED, None, None]
    );
}

#[test]
fn only_a_shutdown_under_way_ends_the_observation() {
    assert_eq!(
        apply_all(
            &mut reporting(),
            &[
                Signal::PrepareForShutdown(false),
                Signal::PrepareForShutdown(true)
            ]
        ),
        [None, Some(Step::Ended)]
    );
}

#[test]
fn the_screensaver_reports_the_lock_until_locked_hint_is_seen_true() {
    let mut tracker = tracker(false, true, Some(false));

    assert_eq!(
        tracker.snapshot(),
        snapshot(SessionState::Unlocked, LockReporting::Live)
    );
    assert_eq!(
        apply_all(
            &mut tracker,
            &[
                Signal::ScreenSaverActive(true),
                Signal::ScreenSaverActive(false),
                Signal::ScreenSaverActive(true),
                locked_hint(true),
                locked_hint(false),
                Signal::ScreenSaverActive(false),
                Signal::ScreenSaverActive(true),
            ]
        ),
        [LOCKED, UNLOCKED, LOCKED, None, UNLOCKED, None, None]
    );
}

#[test]
fn without_a_lock_signal_the_lock_is_unknown_and_only_a_switch_of_the_session_is_recorded() {
    let mut tracker = tracker(false, true, None);

    assert_eq!(
        tracker.snapshot(),
        snapshot(SessionState::Unknown, LockReporting::Unavailable)
    );
    assert_eq!(
        apply_all(
            &mut tracker,
            &[
                locked_hint(false),
                active(true),
                active(false),
                active(true)
            ]
        ),
        [None, None, LOCKED, UNLOCKED]
    );
    assert_eq!(
        tracker.snapshot(),
        snapshot(SessionState::Unknown, LockReporting::Unavailable)
    );
}

#[test]
fn a_lock_signal_that_appears_later_makes_the_lock_reported() {
    let mut by_hint = tracker(false, true, None);
    let mut by_screensaver = tracker(false, true, None);

    assert_eq!(
        (
            by_hint.apply(locked_hint(true)),
            by_screensaver.apply(Signal::ScreenSaverActive(false))
        ),
        (LOCKED, UNLOCKED)
    );
    assert_eq!(
        (by_hint.snapshot(), by_screensaver.snapshot()),
        (
            snapshot(SessionState::Locked, LockReporting::Live),
            snapshot(SessionState::Unlocked, LockReporting::Live)
        )
    );
}

#[test]
fn the_snapshot_follows_the_readings() {
    let cases = [
        (
            (true, true, None),
            snapshot(SessionState::Locked, LockReporting::Live),
        ),
        (
            (true, false, Some(false)),
            snapshot(SessionState::Locked, LockReporting::Live),
        ),
        (
            (false, true, Some(true)),
            snapshot(SessionState::Locked, LockReporting::Live),
        ),
        (
            (false, false, Some(false)),
            snapshot(SessionState::Locked, LockReporting::Live),
        ),
        (
            (false, true, Some(false)),
            snapshot(SessionState::Unlocked, LockReporting::Live),
        ),
        (
            (false, false, None),
            snapshot(SessionState::Unknown, LockReporting::Unavailable),
        ),
    ];

    for ((locked_hint, active, screensaver), expected) in cases {
        assert_eq!(
            tracker(locked_hint, active, screensaver).snapshot(),
            expected,
            "LockedHint {locked_hint}, Active {active}, screensaver {screensaver:?}"
        );
    }
}

#[test]
fn a_session_read_locked_records_its_unlock_first() {
    let mut tracker = tracker(true, true, Some(true));

    assert_eq!(
        apply_all(
            &mut tracker,
            &[Signal::ScreenSaverActive(false), locked_hint(false)]
        ),
        [None, UNLOCKED]
    );
}
