use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::Instant;

use jiff::SignedDuration;

use super::screensaver::{Read, UNIT_CHECK_STEP, unit};
use super::*;

use Backend::*;

/// What a desktop answers to each probe: the backends present, and the answers the screensaver
/// gives its unit check when it runs.
#[derive(Clone, Copy, Default)]
struct Desktop {
    ext_idle: bool,
    mutter: bool,
    muffin: bool,
    screensaver: Option<&'static [u32]>,
    xscreensaver: bool,
}

impl Desktop {
    fn answers(self, backend: Backend) -> bool {
        match backend {
            ExtIdleNotify => self.ext_idle,
            MutterIdleMonitor => self.mutter,
            MuffinIdleMonitor => self.muffin,
            ScreenSaver => self.screensaver.is_some_and(|values| {
                let reads: Vec<Read> = (0..)
                    .zip(values)
                    .map(|(index, &value)| Read {
                        at: UNIT_CHECK_STEP * index,
                        value,
                    })
                    .collect();
                unit(&reads).is_some()
            }),
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
const SCREENSAVER_IN_MILLISECONDS: Option<&[u32]> = Some(&[
    60_000, 60_100, 60_200, 60_300, 60_400, 60_500, 60_600, 60_700, 60_800, 60_900, 61_000,
]);
const SCREENSAVER_IN_SECONDS: Option<&[u32]> = Some(&[60, 60, 60, 61, 61, 61, 61, 61, 61, 61, 61]);
const SCREENSAVER_THAT_STANDS_STILL: Option<&[u32]> = Some(&[0; 11]);

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

/// How long a stuck backend waits for the test before it gives up: a read of the watch that
/// waited on it would take that long.
const STUCK: Duration = Duration::from_secs(10);

/// A desktop whose backend, idle for 42 s at every read, opens once `opening` says so, as the
/// screensaver's unit check takes a second, and whose reads after the first say so on `stuck`
/// and then wait for `reading`, as a bus that hangs.
struct Stuck {
    opening: mpsc::Receiver<()>,
    reader: Option<StuckReader>,
}

struct StuckReader {
    reads: usize,
    stuck: mpsc::Sender<()>,
    reading: mpsc::Receiver<()>,
}

impl Backends for Stuck {
    fn open(&mut self, _backend: Backend) -> Option<Opened> {
        let _ = self.opening.recv_timeout(STUCK);
        Some(Opened {
            reader: Box::new(self.reader.take()?),
            inhibitor_blind: true,
        })
    }
}

impl IdleReader for StuckReader {
    fn idle(&mut self, _now: Duration) -> Option<Duration> {
        self.reads += 1;
        if self.reads > 1 {
            let _ = self.stuck.send(());
            let _ = self.reading.recv_timeout(STUCK);
        }
        Some(Duration::from_secs(42))
    }
}

/// The watch of a Wayland session on `Stuck` at second 1 000, what it logged, and the ends the
/// test holds: `open` lets the backend open, `stuck` hears each read that hangs, and dropping
/// `release` ends them.
struct StuckWatch {
    watch: IdleWatch<Clock>,
    notes: Arc<Mutex<Vec<IdleNote>>>,
    open: mpsc::Sender<()>,
    stuck: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

fn stuck_watch(clock: &Clock) -> StuckWatch {
    clock.set(1_000);
    let (open, opening) = mpsc::channel();
    let (stuck_sender, stuck) = mpsc::channel();
    let (release, reading) = mpsc::channel();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let log = {
        let notes = Arc::clone(&notes);
        move |note| notes.lock().unwrap().push(note)
    };
    let backends = Stuck {
        opening,
        reader: Some(StuckReader {
            reads: 0,
            stuck: stuck_sender,
            reading,
        }),
    };
    let watch = IdleWatch::new(WAYLAND, backends, clock.clone(), log).unwrap();
    StuckWatch {
        watch,
        notes,
        open,
        stuck,
        release,
    }
}

/// The first reading of `watch`, once its thread has read the backend.
fn first_reading(watch: &IdleWatch<Clock>) -> LastInputReading {
    let deadline = Instant::now() + STUCK;
    loop {
        if let Some(reading) = watch.read() {
            return reading;
        }
        assert!(
            Instant::now() < deadline,
            "the watch never read the backend"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `read` and how long it took.
fn timed(watch: &IdleWatch<Clock>) -> (Option<LastInputReading>, Duration) {
    let asked = Instant::now();
    (watch.read(), asked.elapsed())
}

#[test]
fn a_read_of_the_watch_waits_neither_on_the_selection_nor_on_the_backend() {
    let clock = Clock::default();
    let started = Instant::now();
    let StuckWatch {
        watch,
        notes,
        open,
        stuck,
        release: _release,
    } = stuck_watch(&clock);
    let made_in = started.elapsed();
    let (while_selecting, selecting_read_in) = timed(&watch);

    open.send(()).unwrap();
    let first = first_reading(&watch);
    stuck.recv_timeout(STUCK).expect("the second read hangs");
    clock.set(1_007);
    let (while_stuck, stuck_read_in) = timed(&watch);

    for (what, took) in [
        ("making the watch", made_in),
        ("a read during the selection", selecting_read_in),
        ("a read while the backend hangs", stuck_read_in),
    ] {
        assert!(took < STUCK / 2, "{what} took {took:?}");
    }
    assert_eq!(
        (while_selecting, first, while_stuck),
        (
            None,
            reading_from_idle(Duration::from_secs(1_000), Duration::from_secs(42)),
            Some(reading_from_idle(
                Duration::from_secs(1_007),
                Duration::from_secs(49)
            ))
        ),
        "the idle time at the instant of the read, from the last input the thread read"
    );
    assert_eq!(taken(&notes), [IdleNote::Selected(Some(ExtIdleNotify))]);
}

#[test]
fn dropping_the_watch_ends_its_thread_and_closes_the_backend() {
    let clock = Clock::default();
    let StuckWatch {
        watch,
        open,
        stuck,
        release,
        ..
    } = stuck_watch(&clock);
    open.send(()).unwrap();
    first_reading(&watch);

    drop(watch);
    drop(release);

    let deadline = Instant::now() + STUCK;
    loop {
        match stuck.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(()) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("the watch's thread still runs"),
        }
    }
}
