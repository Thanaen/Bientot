//! What the real app does with the notification server of `scripts/linux-desktop-harness.sh`,
//! python-dbusmock's: it asks the server at start whether it answers, and when it does not, the
//! diagnostic log says the notifications are unavailable; when no window can open, it says why
//! through the server and on the standard error, then quits. Each test starts the `worktime`
//! executable on a data folder of its own. The tests share the session's bus, so they take
//! turns.
#![cfg(target_os = "linux")]

use std::io::Read as _;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
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

/// The summary and body of each `Notify` the server mock was sent, in order.
fn notified() -> Vec<(String, String)> {
    let calls: serde_json::Value = serde_json::from_str(&busctl(&[
        "--json=short",
        "call",
        SERVER,
        SERVER_PATH,
        "org.freedesktop.DBus.Mock",
        "GetMethodCalls",
        "s",
        "Notify",
    ]))
    .unwrap();
    calls["data"][0]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| {
            let text = |index: usize| call[1][index]["data"].as_str().unwrap().to_owned();
            (text(3), text(4))
        })
        .collect()
}

const NO_GRAPHICS_FR: &str = "Bientôt n’a pas pu ouvrir de fenêtre et s’est fermé. Vérifiez que les \
     pilotes graphiques de votre système sont installés, puis relancez l’application.";
const NO_GRAPHICS_EN: &str = "Bientôt could not open a window and has closed. Check that your system’s \
     graphics drivers are installed, then restart the app.";
const START_FAILED: &str =
    "Bientôt could not start. Check the local settings and database, then restart the app.";

/// What a start without any graphics driver left: neither Vulkan nor EGL finds one.
struct Stopped {
    status: ExitStatus,
    stderr: String,
    /// Whether it wrote the ready file the CI startup measurement waits for.
    ready: bool,
}

impl Stopped {
    fn first_line(&self) -> &str {
        self.stderr.lines().next().unwrap_or_default()
    }
}

/// Starts the app without graphics on a data folder `prepare` fills, and waits for it to end.
fn start_without_graphics(arguments: &[&str], prepare: impl FnOnce(&Path)) -> Stopped {
    let folder = TempDir::new().unwrap();
    let data = folder.path().join("data");
    std::fs::create_dir(&data).unwrap();
    prepare(&data);
    let ready = folder.path().join("ready");
    let mut child = Command::new(env!("CARGO_BIN_EXE_worktime"))
        .args(arguments)
        .arg("--data-dir")
        .arg(&data)
        .env("BIENTOT_READY_FILE", &ready)
        .env("VK_ICD_FILENAMES", folder.path().join("no-vulkan.json"))
        .env(
            "__EGL_VENDOR_LIBRARY_FILENAMES",
            folder.path().join("no-egl.json"),
        )
        .env("EGL_LOG_LEVEL", "fatal")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app runs on without a window");
        }
        sleep(Duration::from_millis(100));
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    Stopped {
        status,
        stderr,
        ready: ready.exists(),
    }
}

fn settings(data: &Path) {
    std::fs::write(data.join("settings.json"), "{}").unwrap();
}

/// `text` as the server mock is sent it, under the app's name.
fn under_the_name(text: &str) -> Vec<(String, String)> {
    vec![("Bientôt".to_owned(), text.to_owned())]
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_without_graphics_the_flyout_says_why_and_the_app_quits() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let before = notified().len();

    let stopped = start_without_graphics(&["--open-flyout", "--french"], settings);

    assert_eq!(stopped.status.code(), Some(1), "{}", stopped.stderr);
    assert_eq!(stopped.stderr, format!("{NO_GRAPHICS_FR}\n"));
    assert_eq!(notified()[before..], under_the_name(NO_GRAPHICS_FR)[..]);
    assert!(!stopped.ready, "the ready file is written");
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_without_graphics_the_onboarding_says_why_and_the_app_quits() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let before = notified().len();

    let stopped = start_without_graphics(&["--english"], |_| {});

    assert_eq!(stopped.status.code(), Some(1), "{}", stopped.stderr);
    assert_eq!(stopped.stderr, format!("{NO_GRAPHICS_EN}\n"));
    assert_eq!(notified()[before..], under_the_name(NO_GRAPHICS_EN)[..]);
    assert!(!stopped.ready, "the ready file is written");
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_without_graphics_a_failed_start_keeps_its_own_text() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let before = notified().len();

    let stopped = start_without_graphics(&["--english"], |data| {
        settings(data);
        std::fs::write(data.join("worktime.db"), [0x5a; 4096]).unwrap();
    });

    assert_eq!(stopped.status.code(), Some(1), "{}", stopped.stderr);
    assert_eq!(stopped.first_line(), START_FAILED);
    assert_eq!(notified()[before..], under_the_name(START_FAILED)[..]);
    assert!(!stopped.ready, "the ready file is written");
}
