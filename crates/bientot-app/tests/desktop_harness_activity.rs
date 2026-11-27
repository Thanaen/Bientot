//! What the real app reads of the session `scripts/linux-desktop-harness.sh` starts: the idle time
//! of the session's first backend, `MIT-SCREEN-SAVER` under Xvfb or ext-idle under Sway, which
//! the built-in source turns into activity in the ledger; and the lock and the sleep of a session
//! of python-dbusmock's logind. Each test starts the `worktime` executable hidden, on a data folder
//! of its own. The tests share the buses, so they take turns.
#![cfg(target_os = "linux")]

use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use tempfile::TempDir;

static HARNESS_TURN: Mutex<()> = Mutex::new(());

const WAIT: Duration = Duration::from_secs(60);
const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MOCK: &str = "org.freedesktop.DBus.Mock";

/// Polls `check` until it gives a value, for up to `wait`.
fn eventually<T>(what: &str, wait: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "gave up waiting for {what}");
        sleep(Duration::from_millis(200));
    }
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// A key press or a pointer move, as the session's display server sees one.
fn input() {
    let (program, arguments): (&str, &[&str]) = if wayland() {
        ("wtype", &["-k", "Shift_L"])
    } else {
        ("xdotool", &["mousemove_relative", "--", "1", "1"])
    };
    let status = Command::new(program)
        .args(arguments)
        .status()
        .unwrap_or_else(|error| panic!("{program} did not run: {error}"));
    assert!(status.success(), "{program} failed: {status}");
}

/// Calls the logind mock on the harness's system bus.
fn logind(path: &str, method: &str, arguments: &[&str]) {
    let output = Command::new("busctl")
        .args(["--system", "call", LOGIND, path, MOCK, method])
        .args(arguments)
        .output()
        .expect("busctl runs");
    assert!(
        output.status.success(),
        "{method}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The app, started hidden on a data folder of its own with the built-in source selected;
/// killed when dropped.
struct App {
    child: Child,
    folder: TempDir,
}

impl App {
    fn start(environment: &[(&str, &str)]) -> Self {
        let folder = TempDir::new().unwrap();
        let data = folder.path().join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(
            data.join("settings.json"),
            r#"{"activitySource":"built-in"}"#,
        )
        .unwrap();
        let ready = folder.path().join("ready");
        let child = Command::new(env!("CARGO_BIN_EXE_worktime"))
            .args(["--worktime-windows-startup", "--data-dir"])
            .arg(&data)
            .env("BIENTOT_READY_FILE", &ready)
            .envs(environment.iter().copied())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let app = Self { child, folder };
        eventually("the app", WAIT, || ready.exists().then_some(()));
        app
    }

    /// The diagnostic log so far.
    fn log(&self) -> String {
        std::fs::read_dir(self.folder.path().join("data/logs"))
            .map(|files| {
                files
                    .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The values of `column` in `table` of the ledger, oldest first.
    fn ledger(&self, table: &str, column: &str) -> Vec<String> {
        let ledger = Connection::open_with_flags(
            self.folder.path().join("data/worktime.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        ledger.busy_timeout(Duration::from_secs(5)).unwrap();
        let mut rows = ledger
            .prepare(&format!("SELECT {column} FROM {table} ORDER BY Id"))
            .unwrap();
        rows.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_counts_the_input_the_session_s_first_idle_backend_reads() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let app = App::start(&[]);

    let backend = if wayland() {
        "ext-idle-notify-v1"
    } else {
        "MIT-SCREEN-SAVER"
    };
    let selected = format!(r#""Event":"idle.backend","Properties":{{"name":"{backend}"}}"#);
    eventually("the idle backend in the log", WAIT, || {
        app.log().contains(&selected).then_some(())
    });
    input();
    eventually("an active span in the ledger", WAIT, || {
        let states = app.ledger("ActivitySpan", "State");
        states.iter().any(|state| state == "active").then_some(())
    });
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_records_the_lock_and_the_sleep_of_the_logind_session() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let id = format!("app{}", std::process::id());
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
    logind(
        MANAGER_PATH,
        "AddSession",
        &["ssusb", &id, "seat0", uid.trim(), "harness", "true"],
    );
    let session = format!("/org/freedesktop/login1/session/{id}");
    let set = |locked: &str| {
        logind(
            &session,
            "UpdateProperties",
            &[
                "sa{sv}",
                "org.freedesktop.login1.Session",
                "1",
                "LockedHint",
                "b",
                locked,
            ],
        );
    };
    let app = App::start(&[("XDG_SESSION_ID", &id)]);
    let at_start = app.ledger("SessionEvent", "Kind").len();
    let recorded = |count: usize| {
        eventually("the session events in the ledger", WAIT, || {
            let kinds = app.ledger("SessionEvent", "Kind");
            (kinds.len() >= at_start + count).then(|| kinds[at_start..].to_vec())
        })
    };

    set("true");
    recorded(1);
    set("false");
    recorded(2);
    logind(
        MANAGER_PATH,
        "EmitSignal",
        &[
            "sssav",
            "org.freedesktop.login1.Manager",
            "PrepareForSleep",
            "b",
            "1",
            "b",
            "true",
        ],
    );

    assert_eq!(recorded(3), ["locked", "unlocked", "suspend"]);
}
