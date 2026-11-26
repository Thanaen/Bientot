//! One Bientôt per data folder with real processes in `scripts/linux-desktop-harness.sh`: a later
//! `worktime` on the folder of a running one hands over and exits, and the running one shows its
//! flyout. The x11 session has no `XDG_RUNTIME_DIR`, so its lock and socket are in the data
//! folder; the wayland session has one. The tests share the session's bus and display, so they
//! take turns.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tempfile::TempDir;

static HARNESS_TURN: Mutex<()> = Mutex::new(());

const WAIT: Duration = Duration::from_secs(60);
const REDIRECTED: &str = "instance.redirected";

/// Polls `check` until it gives a value, for up to `wait`.
fn eventually<T>(what: &str, wait: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "gave up waiting for {what}");
        sleep(Duration::from_millis(100));
    }
}

/// A data folder with settings saved, so that no onboarding shows.
struct Folder {
    root: TempDir,
}

impl Folder {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let data = root.path().join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("settings.json"), "{}").unwrap();
        Self { root }
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_worktime"));
        command
            .args(arguments)
            .arg("--data-dir")
            .arg(self.data())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// The app, started hidden as at sign-in, once its shell is ready.
    fn start(&self) -> App {
        let ready = self.root.path().join("ready");
        std::fs::remove_file(&ready).ok();
        let child = self
            .command(&["--autostart"])
            .env("BIENTOT_READY_FILE", &ready)
            .spawn()
            .unwrap();
        eventually("the app to start", WAIT, || ready.exists().then_some(()));
        App { child }
    }

    /// A launch by the user on the folder, run to its end.
    fn launch(&self) -> ExitStatus {
        let mut launch = App {
            child: self.command(&[]).spawn().unwrap(),
        };
        eventually("the later launch to exit", WAIT, || {
            launch.child.try_wait().unwrap()
        })
    }

    fn log(&self) -> String {
        std::fs::read_dir(self.data().join("logs"))
            .map(|files| {
                files
                    .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many later launches the running app showed itself for.
    fn redirections(&self) -> usize {
        self.log().matches(REDIRECTED).count()
    }

    fn screenshot_folder(&self) -> &Path {
        self.root.path()
    }
}

/// The app; killed when dropped.
struct App {
    child: Child,
}

impl App {
    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// Ends it as `kill -9` does.
    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Whether the app shows something on screen: on X11 a viewable window of its process, on
/// Wayland pixels other than sway's background, which fills the output's centre.
fn shows_something(app: &App, folder: &Folder) -> bool {
    if !wayland() {
        return Command::new("xdotool")
            .args(["search", "--onlyvisible", "--pid"])
            .arg(app.child.id().to_string())
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success();
    }
    let file = folder.screenshot_folder().join("screen.ppm");
    let grabbed = Command::new("grim")
        .args(["-t", "ppm"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(grabbed.success(), "grim");
    let bytes = std::fs::read(file).unwrap();
    // P6, width, height, the maximum, each followed by one whitespace.
    let mut fields = bytes.splitn(5, u8::is_ascii_whitespace);
    let _magic = fields.next();
    let number = |field: Option<&[u8]>| -> usize {
        std::str::from_utf8(field.unwrap())
            .unwrap()
            .parse()
            .unwrap()
    };
    let (width, height) = (number(fields.next()), number(fields.next()));
    let _maximum = fields.next();
    let pixels = fields.next().unwrap();
    let centre = &pixels[(height / 2 * width + width / 2) * 3..][..3];
    pixels.chunks_exact(3).any(|pixel| pixel != centre)
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_a_later_launch_hands_over_and_the_running_app_shows_its_flyout() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = Folder::new();
    let mut first = folder.start();
    sleep(Duration::from_secs(1));
    assert!(
        !shows_something(&first, &folder),
        "started at sign-in, the app shows nothing"
    );
    assert_eq!(folder.redirections(), 0);

    let started = Instant::now();
    assert!(folder.launch().success(), "the later launch exits with 0");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "answered, not left to its deadline: {:?}",
        started.elapsed()
    );
    assert!(first.alive(), "the running app stays");
    eventually("the redirection in the log", WAIT, || {
        (folder.redirections() == 1).then_some(())
    });
    eventually("the flyout", WAIT, || {
        shows_something(&first, &folder).then_some(())
    });
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_after_kill_9_the_next_launch_holds_the_folder_and_takes_later_ones() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = Folder::new();
    folder.start().kill();

    let mut next = folder.start();
    assert!(next.alive(), "the lock died with the killed app");
    assert!(folder.launch().success());
    assert!(next.alive());
    eventually("the redirection in the log", WAIT, || {
        (folder.redirections() == 1).then_some(())
    });
}
