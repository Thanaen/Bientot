//! What the real app does with the notification server of `scripts/linux-desktop-harness.sh`,
//! python-dbusmock's: it asks the server at start whether it answers, and when it does not, the
//! diagnostic log says the notifications are unavailable. Each test starts the `worktime`
//! executable hidden, on a data folder of its own. The tests share the session's bus, so they
//! take turns.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tempfile::TempDir;

static HARNESS_TURN: Mutex<()> = Mutex::new(());

const SERVER: &str = "org.freedesktop.Notifications";
const SERVER_PATH: &str = "/org/freedesktop/Notifications";
const WAIT: Duration = Duration::from_secs(60);
const REFUSED: &str = "notification.registration_failed";

fn busctl(arguments: &[&str]) -> String {
    let output = Command::new("busctl")
        .arg("--user")
        .args(arguments)
        .output()
        .expect("busctl runs");
    assert!(
        output.status.success(),
        "busctl {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// How many times the server mock was asked `GetServerInformation`.
fn information_asked() -> usize {
    let calls = busctl(&[
        "call",
        SERVER,
        SERVER_PATH,
        "org.freedesktop.DBus.Mock",
        "GetMethodCalls",
        "s",
        "GetServerInformation",
    ]);
    calls
        .trim()
        .trim_start_matches("a(tav) ")
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Has the server mock answer `GetServerInformation` with `code`, python-dbusmock's method body.
fn answer_information_with(code: &str) {
    busctl(&[
        "call",
        SERVER,
        SERVER_PATH,
        "org.freedesktop.DBus.Mock",
        "AddMethod",
        "sssss",
        SERVER,
        "GetServerInformation",
        "",
        "ssss",
        code,
    ]);
}

/// The server mock refusing `GetServerInformation` for the life of the value.
struct Refusing;

impl Refusing {
    fn new() -> Self {
        answer_information_with(
            "raise dbus.exceptions.DBusException('refused', name='org.freedesktop.DBus.Error.Failed')",
        );
        Self
    }
}

impl Drop for Refusing {
    fn drop(&mut self) {
        answer_information_with(r#"ret = ("mock-notify", "test vendor", "1.0", "1.1")"#);
    }
}

/// The app, started hidden on a data folder of its own; killed when dropped.
struct App {
    child: Child,
    folder: TempDir,
}

impl App {
    fn start() -> Self {
        let folder = TempDir::new().unwrap();
        let data = folder.path().join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("settings.json"), "{}").unwrap();
        let ready = folder.path().join("ready");
        let child = Command::new(env!("CARGO_BIN_EXE_worktime"))
            .args(["--worktime-windows-startup", "--data-dir"])
            .arg(&data)
            .env("BIENTOT_READY_FILE", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let app = Self { child, folder };
        let deadline = Instant::now() + WAIT;
        while !ready.exists() {
            assert!(Instant::now() < deadline, "gave up waiting for the app");
            sleep(Duration::from_millis(100));
        }
        app
    }

    /// The diagnostic log so far.
    fn log(&self) -> String {
        read_logs(&self.folder.path().join("data/logs"))
    }
}

fn read_logs(folder: &Path) -> String {
    std::fs::read_dir(folder)
        .map(|files| {
            files
                .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default()
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_logs_the_notifications_unavailable_only_when_no_server_answers() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);

    let asked = information_asked();
    let answered = App::start();
    assert_eq!(
        information_asked(),
        asked + 1,
        "the app asks the server once"
    );
    assert!(!answered.log().contains(REFUSED), "{}", answered.log());
    drop(answered);

    let refusing = Refusing::new();
    let refused = App::start();
    drop(refusing);
    assert!(refused.log().contains(REFUSED), "{}", refused.log());
}
