//! Against the session `scripts/linux-desktop-harness.sh` starts: Xvfb or headless Sway, and on
//! the session bus the Mutter idle monitor of `tests/support/mutter_idle_monitor.py`. Input comes
//! from `xdotool` on X11 and `wtype` on Wayland. The ScreenSaver and Muffin services are served
//! by the tests themselves, on the shared session bus, so the tests take turns.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, mpsc};
use std::time::Instant;

use bientot_platform::activity::LastInputProbe;
use zbus::Connection;

use super::*;
use crate::idle::screensaver::Unit;

/// The idle time the probe reads now.
fn idle(probe: &impl LastInputProbe) -> Duration {
    let reading = probe.read().expect("the probe reads the idle time");
    reading.idle_duration().unsigned_abs()
}

/// A key press or a pointer move, as the session's display server sees one.
fn input(session: SessionType) {
    let (program, arguments): (&str, &[&str]) = match session {
        SessionType::X11 => ("xdotool", &["mousemove_relative", "--", "1", "1"]),
        SessionType::Wayland => ("wtype", &["-k", "Shift_L"]),
    };
    let status = Command::new(program)
        .args(arguments)
        .status()
        .unwrap_or_else(|error| panic!("{program} did not run: {error}"));
    assert!(status.success(), "{program} failed: {status}");
}

fn noted() -> (mpsc::Receiver<IdleNote>, impl Fn(IdleNote) + Send + 'static) {
    let (sender, received) = mpsc::channel();
    (received, move |note| {
        let _ = sender.send(note);
    })
}

fn assert_near(read: Duration, expected: Duration, slack: Duration) {
    assert!(
        read >= expected && read <= expected + slack,
        "read {read:?}, expected {expected:?} to {:?}",
        expected + slack
    );
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_selects_the_first_backend_of_the_session_and_follows_its_input() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let session = this_session().expect("the harness runs a display server");
    let (notes, log) = noted();
    let probe = IdleProbe::of_this_session(log);
    let expected = match session {
        SessionType::X11 => vec![IdleNote::Selected(Some(Backend::XScreenSaver))],
        // Sway 1.9 offers version 1 of the protocol only, which counts inhibitors (research.md,
        // R6); later Sways offer version 2, which does not.
        SessionType::Wayland => {
            let opened = SystemBackends
                .open(Backend::ExtIdleNotify)
                .expect("Sway offers ext-idle");
            let mut notes = vec![IdleNote::Selected(Some(Backend::ExtIdleNotify))];
            if !opened.inhibitor_blind {
                notes.push(IdleNote::InhibitorsCount);
            }
            notes
        }
    };
    assert_eq!(notes.try_iter().collect::<Vec<_>>(), expected);

    input(session);
    let typed = Instant::now();
    std::thread::sleep(Duration::from_millis(6_500));
    assert_near(
        idle(&probe),
        typed.elapsed() - Duration::from_millis(500),
        Duration::from_secs(1),
    );

    input(session);
    std::thread::sleep(Duration::from_millis(200));
    assert!(idle(&probe) < Duration::from_secs(1));
}

/// Calls the mock interface of the Mutter idle monitor mock.
fn set_mutter_idle_time(idle: Duration) {
    async_io::block_on(async {
        let bus = Connection::session().await?;
        bus.call_method(
            Some("org.gnome.Mutter.IdleMonitor"),
            "/org/gnome/Mutter/IdleMonitor/Core",
            Some("org.freedesktop.DBus.Mock"),
            "SetIdletime",
            &(idle.as_millis() as u64,),
        )
        .await
    })
    .expect("the Mutter idle monitor mock runs");
}

/// The backends of this machine, but those of `missing`, as on a desktop that lacks them.
struct Without<const N: usize> {
    missing: [Backend; N],
}

impl<const N: usize> Backends for Without<N> {
    fn open(&mut self, backend: Backend) -> Option<Opened> {
        if self.missing.contains(&backend) {
            return None;
        }
        SystemBackends.open(backend)
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_reads_mutter_s_idle_monitor_where_wayland_has_no_ext_idle() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let (notes, log) = noted();
    let probe = IdleProbe::new(
        Some(SessionType::Wayland),
        Without {
            missing: [Backend::ExtIdleNotify],
        },
        Monotonic,
        log,
    );
    assert_eq!(probe.backend(), Some(Backend::MutterIdleMonitor));
    assert_eq!(
        notes.try_iter().collect::<Vec<_>>(),
        [IdleNote::Selected(Some(Backend::MutterIdleMonitor))]
    );

    // The probe counts back no further than the boot, and a runner may have booted minutes ago.
    let long = Duration::from_secs((Monotonic.now() / 2).as_secs().min(600));
    set_mutter_idle_time(long);
    assert_near(idle(&probe), long, Duration::from_millis(500));
    set_mutter_idle_time(Duration::from_millis(1_250));
    assert_near(
        idle(&probe),
        Duration::from_millis(1_250),
        Duration::from_millis(500),
    );
}

/// How a desktop's idle service counts.
#[derive(Clone, Copy)]
enum Count {
    /// From its start, in a unit.
    In(Unit),
    StandsStill,
    /// In milliseconds, but input resets the count shortly before every read: 5, then 6, then 7.
    ResetBeforeEachRead,
}

/// A desktop's idle service on the session bus: `GetIdletime` or `GetSessionIdleTime`.
struct Counter {
    since: Instant,
    count: Count,
    reads: AtomicU64,
}

impl Counter {
    fn value(&self) -> u64 {
        let elapsed = self.since.elapsed() + Duration::from_secs(90);
        match self.count {
            Count::In(Unit::Milliseconds) => elapsed.as_millis() as u64,
            Count::In(Unit::Seconds) => elapsed.as_secs(),
            Count::StandsStill => 7,
            Count::ResetBeforeEachRead => 5 + self.reads.fetch_add(1, Ordering::Relaxed),
        }
    }
}

struct ScreenSaverService(Counter);

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaverService {
    fn get_session_idle_time(&self) -> u32 {
        self.0.value() as u32
    }
}

struct MuffinService(Counter);

#[zbus::interface(name = "org.cinnamon.Muffin.IdleMonitor")]
impl MuffinService {
    fn get_idletime(&self) -> u64 {
        self.0.value()
    }
}

/// A service on the session bus, whose name is released when it is dropped.
struct Served {
    connection: Connection,
    name: &'static str,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = async_io::block_on(self.connection.release_name(self.name));
    }
}

fn serve<I: zbus::object_server::Interface>(name: &'static str, path: &str, service: I) -> Served {
    let connection = async_io::block_on(async {
        zbus::connection::Builder::session()?
            .name(name)?
            .serve_at(path, service)?
            .build()
            .await
    })
    .unwrap_or_else(|error| panic!("{name} could not be served: {error}"));
    Served { connection, name }
}

fn counter(count: Count) -> Counter {
    Counter {
        since: Instant::now(),
        count,
        reads: AtomicU64::new(0),
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_reads_the_screensaver_in_the_unit_it_answers_in() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let open = |path, count| {
        let _service = serve(
            "org.freedesktop.ScreenSaver",
            path,
            ScreenSaverService(counter(count)),
        );
        SystemBackends
            .open(Backend::ScreenSaver)
            .map(|mut opened| opened.reader.idle(Duration::ZERO))
    };

    let milliseconds = open(
        "/org/freedesktop/ScreenSaver",
        Count::In(Unit::Milliseconds),
    );
    let seconds = open("/ScreenSaver", Count::In(Unit::Seconds));
    let still = open("/ScreenSaver", Count::StandsStill);
    let reset = open("/ScreenSaver", Count::ResetBeforeEachRead);

    assert_near(
        milliseconds.flatten().unwrap(),
        Duration::from_secs(91),
        Duration::from_millis(500),
    );
    assert_eq!(seconds, Some(Some(Duration::from_secs(91))));
    assert_eq!(still.map(|_| "opened"), None);
    assert_eq!(reset.map(|_| "opened"), None);
    assert!(
        SystemBackends.open(Backend::ScreenSaver).is_none(),
        "no screensaver runs"
    );
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_reads_muffin_s_idle_monitor() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert!(SystemBackends.open(Backend::MuffinIdleMonitor).is_none());
    let _service = serve(
        "org.cinnamon.Muffin.IdleMonitor",
        "/org/cinnamon/Muffin/IdleMonitor/Core",
        MuffinService(counter(Count::In(Unit::Milliseconds))),
    );

    let mut opened = SystemBackends
        .open(Backend::MuffinIdleMonitor)
        .expect("Muffin's idle monitor answers");

    assert_near(
        opened.reader.idle(Duration::ZERO).unwrap(),
        Duration::from_secs(90),
        Duration::from_millis(500),
    );
    assert!(opened.inhibitor_blind);
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_selects_again_when_the_backend_stops_answering() {
    let _turn = crate::HARNESS_TURN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let service = serve(
        "org.cinnamon.Muffin.IdleMonitor",
        "/org/cinnamon/Muffin/IdleMonitor/Core",
        MuffinService(counter(Count::In(Unit::Milliseconds))),
    );
    let (notes, log) = noted();
    let probe = IdleProbe::new(
        Some(SessionType::X11),
        Without {
            missing: [Backend::XScreenSaver, Backend::MutterIdleMonitor],
        },
        Monotonic,
        log,
    );
    assert!(probe.read().is_some());

    drop(service);

    assert_eq!(probe.read(), None);
    assert_eq!(probe.backend(), None);
    assert_eq!(
        notes.try_iter().collect::<Vec<_>>(),
        [
            IdleNote::Selected(Some(Backend::MuffinIdleMonitor)),
            IdleNote::Changed {
                from: Some(Backend::MuffinIdleMonitor),
                to: None
            }
        ]
    );
}
