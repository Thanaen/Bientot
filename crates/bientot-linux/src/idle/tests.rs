use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::SignedDuration;

use super::screensaver::{UNIT_CHECK_GAP, unit};
use super::*;

use Backend::*;

/// What a desktop answers to each probe: the backends present, and the two reads of the
/// screensaver's unit check when it runs.
#[derive(Clone, Copy, Default)]
struct Desktop {
    ext_idle: bool,
    mutter: bool,
    muffin: bool,
    screensaver: Option<(u32, u32)>,
    xscreensaver: bool,
}

impl Desktop {
    fn answers(self, backend: Backend) -> bool {
        match backend {
            ExtIdleNotify => self.ext_idle,
            MutterIdleMonitor => self.mutter,
            MuffinIdleMonitor => self.muffin,
            ScreenSaver => self
                .screensaver
                .is_some_and(|(first, second)| unit(first, second, UNIT_CHECK_GAP).is_some()),
            XScreenSaver => self.xscreensaver,
        }
    }
}

/// The backend `session` selects on `desktop`, and the probes run to find it.
fn selected(session: Option<SessionType>, desktop: Desktop) -> (Option<Backend>, Vec<Backend>) {
    let mut probed = Vec::new();
    let chosen = select(session, |backend| {
        probed.push(backend);
        desktop.answers(backend).then_some(())
    });
    (chosen.map(|(backend, ())| backend), probed)
}

const WAYLAND: Option<SessionType> = Some(SessionType::Wayland);
const X11: Option<SessionType> = Some(SessionType::X11);
const SCREENSAVER_IN_MILLISECONDS: Option<(u32, u32)> = Some((60_000, 61_000));
const SCREENSAVER_IN_SECONDS: Option<(u32, u32)> = Some((60, 61));
const SCREENSAVER_THAT_STANDS_STILL: Option<(u32, u32)> = Some((0, 0));

#[test]
fn each_session_selects_the_first_backend_of_its_order_that_answers() {
    let every = Desktop {
        ext_idle: true,
        mutter: true,
        muffin: true,
        screensaver: SCREENSAVER_IN_MILLISECONDS,
        xscreensaver: true,
    };
    let cases = [
        (
            "KDE Plasma, Sway",
            WAYLAND,
            every,
            Some(ExtIdleNotify),
            &[ExtIdleNotify][..],
        ),
        (
            "GNOME on Wayland",
            WAYLAND,
            Desktop {
                mutter: true,
                xscreensaver: true,
                ..Desktop::default()
            },
            Some(MutterIdleMonitor),
            &[ExtIdleNotify, MutterIdleMonitor],
        ),
        (
            "a Wayland screensaver in milliseconds",
            WAYLAND,
            Desktop {
                screensaver: SCREENSAVER_IN_MILLISECONDS,
                muffin: true,
                ..Desktop::default()
            },
            Some(ScreenSaver),
            &[ExtIdleNotify, MutterIdleMonitor, ScreenSaver],
        ),
        (
            "a Wayland screensaver in seconds",
            WAYLAND,
            Desktop {
                screensaver: SCREENSAVER_IN_SECONDS,
                ..Desktop::default()
            },
            Some(ScreenSaver),
            &[ExtIdleNotify, MutterIdleMonitor, ScreenSaver],
        ),
        (
            "a Wayland screensaver that fails the unit check",
            WAYLAND,
            Desktop {
                screensaver: SCREENSAVER_THAT_STANDS_STILL,
                xscreensaver: true,
                ..Desktop::default()
            },
            None,
            &[ExtIdleNotify, MutterIdleMonitor, ScreenSaver],
        ),
        (
            "X11 with MIT-SCREEN-SAVER",
            X11,
            every,
            Some(XScreenSaver),
            &[XScreenSaver],
        ),
        (
            "GNOME on Xorg without MIT-SCREEN-SAVER",
            X11,
            Desktop {
                ext_idle: true,
                mutter: true,
                screensaver: SCREENSAVER_IN_MILLISECONDS,
                ..Desktop::default()
            },
            Some(MutterIdleMonitor),
            &[XScreenSaver, MutterIdleMonitor],
        ),
        (
            "Cinnamon without MIT-SCREEN-SAVER",
            X11,
            Desktop {
                muffin: true,
                screensaver: SCREENSAVER_IN_MILLISECONDS,
                ..Desktop::default()
            },
            Some(MuffinIdleMonitor),
            &[XScreenSaver, MutterIdleMonitor, MuffinIdleMonitor],
        ),
        (
            "X11 without any",
            X11,
            Desktop {
                ext_idle: true,
                screensaver: SCREENSAVER_IN_MILLISECONDS,
                ..Desktop::default()
            },
            None,
            &[XScreenSaver, MutterIdleMonitor, MuffinIdleMonitor],
        ),
        ("no display server", None, every, None, &[]),
    ];

    for (desktop_name, session, desktop, expected, probed) in cases {
        assert_eq!(
            selected(session, desktop),
            (expected, probed.to_vec()),
            "{desktop_name}"
        );
    }
}

/// A clock moved by hand, shared with the test.
#[derive(Clone, Default)]
struct Clock(Arc<AtomicU64>);

impl Clock {
    fn set(&self, seconds: u64) {
        self.0.store(seconds, Ordering::SeqCst);
    }
}

impl MonotonicClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}

/// A reader that answers `idle` until it is told the backend went away.
struct Reader {
    idle: Duration,
    gone: Arc<AtomicBool>,
}

impl IdleReader for Reader {
    fn idle(&mut self, _now: Duration) -> Option<Duration> {
        (!self.gone.load(Ordering::SeqCst)).then_some(self.idle)
    }
}

/// Backends that open when present, each reporting the idle time it was given.
#[derive(Clone, Default)]
struct Machine(Arc<Mutex<MachineState>>);

#[derive(Default)]
struct MachineState {
    present: HashMap<Backend, (Duration, bool)>,
    gone: HashMap<Backend, Arc<AtomicBool>>,
    opened: Vec<Backend>,
}

impl Machine {
    fn with(self, backend: Backend, idle_seconds: u64) -> Self {
        self.add(backend, idle_seconds, true);
        self
    }

    fn add(&self, backend: Backend, idle_seconds: u64, inhibitor_blind: bool) {
        self.state().present.insert(
            backend,
            (Duration::from_secs(idle_seconds), inhibitor_blind),
        );
    }

    /// The backend's bus or compositor goes away: its readers stop answering, and it opens no
    /// more.
    fn remove(&self, backend: Backend) {
        let mut state = self.state();
        state.present.remove(&backend);
        if let Some(gone) = state.gone.remove(&backend) {
            gone.store(true, Ordering::SeqCst);
        }
    }

    fn opened(&self) -> Vec<Backend> {
        std::mem::take(&mut self.state().opened)
    }

    fn state(&self) -> MutexGuard<'_, MachineState> {
        self.0.lock().unwrap()
    }
}

impl Backends for Machine {
    fn open(&mut self, backend: Backend) -> Option<Opened> {
        let mut state = self.state();
        state.opened.push(backend);
        let (idle, inhibitor_blind) = *state.present.get(&backend)?;
        let gone = Arc::new(AtomicBool::new(false));
        state.gone.insert(backend, Arc::clone(&gone));
        Some(Opened {
            reader: Box::new(Reader { idle, gone }),
            inhibitor_blind,
        })
    }
}

/// The probe of `session` on `machine` at second 1 000 of the monotonic clock, and what it
/// logged.
fn probe(
    session: Option<SessionType>,
    machine: &Machine,
    clock: &Clock,
) -> (IdleProbe<Machine, Clock>, Arc<Mutex<Vec<IdleNote>>>) {
    clock.set(1_000);
    let notes = Arc::new(Mutex::new(Vec::new()));
    let log = {
        let notes = Arc::clone(&notes);
        move |note| notes.lock().unwrap().push(note)
    };
    let probe = IdleProbe::new(session, machine.clone(), clock.clone(), log);
    (probe, notes)
}

fn taken(notes: &Mutex<Vec<IdleNote>>) -> Vec<IdleNote> {
    std::mem::take(&mut notes.lock().unwrap())
}

#[test]
fn the_probe_reads_the_selected_backend_at_the_monotonic_instant() {
    let machine = Machine::default()
        .with(MutterIdleMonitor, 42)
        .with(ScreenSaver, 7);
    let clock = Clock::default();
    let (probe, notes) = probe(WAYLAND, &machine, &clock);

    clock.set(1_005);

    assert_eq!(
        probe.read(),
        Some(reading_from_idle(
            Duration::from_secs(1_005),
            Duration::from_secs(42)
        ))
    );
    assert_eq!(probe.backend(), Some(MutterIdleMonitor));
    assert_eq!(taken(&notes), [IdleNote::Selected(Some(MutterIdleMonitor))]);
    assert_eq!(machine.opened(), [ExtIdleNotify, MutterIdleMonitor]);
}

#[test]
fn a_backend_that_counts_inhibitors_is_logged_with_the_selection() {
    let machine = Machine::default();
    machine.add(ExtIdleNotify, 3, false);
    let clock = Clock::default();
    let (_probe, notes) = probe(WAYLAND, &machine, &clock);

    assert_eq!(
        taken(&notes),
        [
            IdleNote::Selected(Some(ExtIdleNotify)),
            IdleNote::InhibitorsCount
        ]
    );
}

#[test]
fn a_backend_that_stops_answering_gives_way_to_the_next_at_once() {
    let machine = Machine::default()
        .with(XScreenSaver, 1)
        .with(MutterIdleMonitor, 2);
    let clock = Clock::default();
    let (probe, notes) = probe(X11, &machine, &clock);
    machine.opened();

    machine.remove(XScreenSaver);
    clock.set(1_005);

    assert_eq!(
        probe.read().map(|reading| reading.idle_duration()),
        Some(SignedDuration::from_secs(2))
    );
    assert_eq!(machine.opened(), [XScreenSaver, MutterIdleMonitor]);
    assert_eq!(
        taken(&notes),
        [
            IdleNote::Selected(Some(XScreenSaver)),
            IdleNote::Changed {
                from: Some(XScreenSaver),
                to: Some(MutterIdleMonitor)
            }
        ]
    );
}

#[test]
fn the_same_backend_coming_back_is_not_a_change() {
    let machine = Machine::default().with(XScreenSaver, 1);
    let clock = Clock::default();
    let (probe, notes) = probe(X11, &machine, &clock);

    machine.remove(XScreenSaver);
    machine.add(XScreenSaver, 4, true);
    clock.set(1_005);

    assert_eq!(
        probe.read().map(|reading| reading.idle_duration()),
        Some(SignedDuration::from_secs(4))
    );
    assert_eq!(taken(&notes), [IdleNote::Selected(Some(XScreenSaver))]);
}

#[test]
fn without_a_backend_the_probe_reads_nothing_and_tries_again_after_a_minute() {
    let machine = Machine::default();
    let clock = Clock::default();
    let (probe, notes) = probe(WAYLAND, &machine, &clock);
    assert_eq!(machine.opened(), order(WAYLAND));

    machine.add(MutterIdleMonitor, 9, true);
    clock.set(1_000 + RETRY.as_secs() - 5);
    let early = probe.read();
    let tried_early = machine.opened();
    clock.set(1_000 + RETRY.as_secs());
    let due = probe.read();

    assert_eq!((early, tried_early), (None, vec![]));
    assert_eq!(
        due.map(|reading| reading.idle_duration()),
        Some(SignedDuration::from_secs(9))
    );
    assert_eq!(
        taken(&notes),
        [
            IdleNote::Selected(None),
            IdleNote::Changed {
                from: None,
                to: Some(MutterIdleMonitor)
            }
        ]
    );
}

#[test]
fn losing_the_only_backend_is_logged_and_reads_nothing() {
    let machine = Machine::default().with(ExtIdleNotify, 1);
    let clock = Clock::default();
    let (probe, notes) = probe(WAYLAND, &machine, &clock);

    machine.remove(ExtIdleNotify);
    clock.set(1_005);

    assert_eq!(probe.read(), None);
    assert_eq!(probe.backend(), None);
    assert_eq!(
        taken(&notes),
        [
            IdleNote::Selected(Some(ExtIdleNotify)),
            IdleNote::Changed {
                from: Some(ExtIdleNotify),
                to: None
            }
        ]
    );
}

#[test]
fn no_display_server_selects_nothing() {
    let machine = Machine::default().with(XScreenSaver, 1);
    let clock = Clock::default();
    let (probe, notes) = probe(None, &machine, &clock);

    assert_eq!(probe.read(), None);
    assert_eq!(machine.opened(), []);
    assert_eq!(taken(&notes), [IdleNote::Selected(None)]);
}

#[test]
fn every_backend_has_a_name_of_its_own() {
    let names: HashSet<_> = [
        ExtIdleNotify,
        MutterIdleMonitor,
        MuffinIdleMonitor,
        ScreenSaver,
        XScreenSaver,
    ]
    .map(Backend::name)
    .into();

    assert_eq!(names.len(), 5);
    assert_eq!(ExtIdleNotify.name(), "ext-idle-notify-v1");
}
