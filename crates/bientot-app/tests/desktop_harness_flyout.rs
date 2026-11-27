//! Where the real app opens its flyout in `scripts/linux-desktop-harness.sh` (T065): under openbox
//! in the corner of the work area a panel leaves, as a layer surface anchored to the click's corner
//! under sway, with the shadow the app draws around it, and as an ordinary window when no tray host
//! shows the icon. Each test starts the
//! `worktime` executable hidden, as at sign-in, on a data folder of its own, and clicks its tray
//! item over D-Bus as a host does. The tests share the session's bus and display, so they take
//! turns.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tempfile::TempDir;

static HARNESS_TURN: Mutex<()> = Mutex::new(());

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WAIT: Duration = Duration::from_secs(60);
/// The gap the app keeps between the flyout and the edges of the area it opens in.
const MARGIN: i32 = 12;
/// The flyout's width at a scale of 1 (`theme::size::FLYOUT_WIDTH`).
const WIDTH: i32 = 360;

fn session() -> &'static str {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "wayland"
    } else {
        "x11"
    }
}

/// Polls `check` until it gives a value, for up to `WAIT`.
fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "gave up waiting for {what}");
        sleep(Duration::from_millis(100));
    }
}

fn run(program: &str, arguments: &[&str]) -> String {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("cannot run {program}: {error}"));
    assert!(
        output.status.success(),
        "{program} {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn busctl(arguments: &[&str]) -> String {
    run("busctl", &[&["--user"], arguments].concat())
}

/// The app, started hidden on a data folder of its own; killed when dropped.
struct App {
    child: Child,
    _folder: TempDir,
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
        let app = Self {
            child,
            _folder: folder,
        };
        eventually("the app to start", || ready.exists().then_some(()));
        app
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// The bus name of its StatusNotifierItem, once the watcher lists it.
    fn item(&self) -> String {
        let prefix = format!("org.kde.StatusNotifierItem-{}-", self.pid());
        eventually("the tray item", || {
            let items = busctl(&[
                "get-property",
                WATCHER,
                "/StatusNotifierWatcher",
                WATCHER,
                "RegisteredStatusNotifierItems",
            ]);
            let start = items.find(&prefix)?;
            let end = items[start..]
                .find(['/', '"'])
                .map_or(items.len(), |end| start + end);
            Some(items[start..end].to_owned())
        })
    }

    /// A primary click on the icon at `x`, `y`, as a host sends it.
    fn click(&self, x: i32, y: i32) {
        busctl(&[
            "call",
            &self.item(),
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
            "Activate",
            "ii",
            &x.to_string(),
            &y.to_string(),
        ]);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The watcher mock away for the life of the value, as on a desktop without a tray host.
struct NoWatcher {
    mock: String,
}

impl NoWatcher {
    fn new() -> Self {
        let owner = busctl(&[
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            WATCHER,
        ]);
        let mock = owner
            .trim()
            .trim_start_matches("s ")
            .trim_matches('"')
            .to_owned();
        busctl(&[
            "call",
            &mock,
            "/StatusNotifierWatcher",
            "org.freedesktop.DBus.Mock",
            "Leave",
        ]);
        Self { mock }
    }
}

impl Drop for NoWatcher {
    fn drop(&mut self) {
        busctl(&[
            "call",
            &self.mock,
            "/StatusNotifierWatcher",
            "org.freedesktop.DBus.Mock",
            "Return",
        ]);
    }
}

mod x11 {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConnectionExt as _, CreateWindowAux, InputFocus, MapState, PropMode, Window,
        WindowClass,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    x11rb::atom_manager! {
        pub Atoms: AtomsCookie {
            _NET_WM_PID,
            _NET_WM_WINDOW_TYPE,
            _NET_WM_WINDOW_TYPE_DOCK,
            _NET_WM_STRUT_PARTIAL,
            _NET_WORKAREA,
            WM_STATE,
        }
    }

    pub struct Display {
        pub connection: RustConnection,
        pub root: Window,
        pub atoms: Atoms,
    }

    /// What the test reads of one of the app's windows.
    #[derive(Debug)]
    pub struct Shown {
        pub id: Window,
        pub x: i32,
        pub y: i32,
        pub width: i32,
        pub height: i32,
        pub override_redirect: bool,
        /// Whether the window manager manages it: it then sets `WM_STATE`.
        pub managed: bool,
    }

    impl Display {
        pub fn open() -> Self {
            let (connection, screen) = RustConnection::connect(None).unwrap();
            let root = connection.setup().roots[screen].root;
            let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
            Self {
                connection,
                root,
                atoms,
            }
        }

        fn cardinals(&self, window: Window, atom: u32, kind: impl Into<u32>) -> Vec<u32> {
            self.connection
                .get_property(false, window, atom, kind, 0, 64)
                .unwrap()
                .reply()
                .map(|reply| reply.value32().map(Iterator::collect).unwrap_or_default())
                .unwrap_or_default()
        }

        /// `_NET_WORKAREA` of the first desktop.
        pub fn work_area(&self) -> Vec<u32> {
            let area = self.cardinals(self.root, self.atoms._NET_WORKAREA, AtomEnum::CARDINAL);
            area.into_iter().take(4).collect()
        }

        /// A dock of `height` pixels across the top or the bottom of a 1920 × 1080 screen, which
        /// the window manager takes off its work area, as a panel's.
        pub fn dock(&self, top: bool, height: u32) -> Window {
            let id = self.connection.generate_id().unwrap();
            let y = if top { 0 } else { 1080 - height as i16 };
            self.connection
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    id,
                    self.root,
                    0,
                    y,
                    1920,
                    height as u16,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    &CreateWindowAux::new(),
                )
                .unwrap();
            let atoms = &self.atoms;
            self.connection
                .change_property32(
                    PropMode::REPLACE,
                    id,
                    atoms._NET_WM_WINDOW_TYPE,
                    AtomEnum::ATOM,
                    &[atoms._NET_WM_WINDOW_TYPE_DOCK],
                )
                .unwrap();
            // left, right, top, bottom, then the start and end of each.
            let strut = if top {
                [0, 0, height, 0, 0, 0, 0, 0, 0, 1919, 0, 0]
            } else {
                [0, 0, 0, height, 0, 0, 0, 0, 0, 0, 0, 1919]
            };
            self.connection
                .change_property32(
                    PropMode::REPLACE,
                    id,
                    atoms._NET_WM_STRUT_PARTIAL,
                    AtomEnum::CARDINAL,
                    &strut,
                )
                .unwrap();
            self.connection.map_window(id).unwrap();
            self.connection.flush().unwrap();
            id
        }

        pub fn destroy(&self, window: Window) {
            self.connection.destroy_window(window).unwrap();
            self.connection.flush().unwrap();
        }

        /// The viewable windows of process `pid`, among the root's children and theirs.
        pub fn shown(&self, pid: u32) -> Vec<Shown> {
            let mut shown = Vec::new();
            let top = self
                .connection
                .query_tree(self.root)
                .unwrap()
                .reply()
                .unwrap();
            for frame in top.children {
                let mut candidates = vec![frame];
                if let Ok(reply) = self.connection.query_tree(frame).unwrap().reply() {
                    candidates.extend(reply.children);
                }
                for id in candidates {
                    let Ok(attributes) = self.connection.get_window_attributes(id).unwrap().reply()
                    else {
                        continue;
                    };
                    let owner = self.cardinals(id, self.atoms._NET_WM_PID, AtomEnum::CARDINAL);
                    if attributes.map_state != MapState::VIEWABLE || owner != [pid] {
                        continue;
                    }
                    let geometry = self.connection.get_geometry(id).unwrap().reply().unwrap();
                    let origin = self
                        .connection
                        .translate_coordinates(id, self.root, 0, 0)
                        .unwrap()
                        .reply()
                        .unwrap();
                    let managed = !self
                        .cardinals(id, self.atoms.WM_STATE, self.atoms.WM_STATE)
                        .is_empty();
                    shown.push(Shown {
                        id,
                        x: origin.dst_x.into(),
                        y: origin.dst_y.into(),
                        width: geometry.width.into(),
                        height: geometry.height.into(),
                        override_redirect: attributes.override_redirect,
                        managed,
                    });
                }
            }
            shown
        }

        /// An ordinary window of another client, mapped, which the window manager manages.
        pub fn other_window(&self) -> Window {
            let id = self.connection.generate_id().unwrap();
            self.connection
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    id,
                    self.root,
                    100,
                    100,
                    400,
                    300,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    &CreateWindowAux::new(),
                )
                .unwrap();
            self.connection.map_window(id).unwrap();
            self.connection.flush().unwrap();
            id
        }

        pub fn give_focus(&self, window: Window) {
            self.connection
                .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
                .unwrap();
            self.connection.flush().unwrap();
        }

        pub fn exists(&self, window: Window) -> bool {
            self.connection
                .get_window_attributes(window)
                .unwrap()
                .reply()
                .is_ok()
        }

        /// Whether the window manager iconified `window`: `WM_STATE` is `IconicState`.
        pub fn iconic(&self, window: Window) -> bool {
            self.cardinals(window, self.atoms.WM_STATE, self.atoms.WM_STATE)
                .first()
                == Some(&3)
        }

        pub fn focus(&self) -> Window {
            self.connection
                .get_input_focus()
                .unwrap()
                .reply()
                .unwrap()
                .focus
        }
    }
}

/// The flyout of `app` once it is the only window it shows and stopped moving.
fn x11_flyout(display: &x11::Display, app: &App) -> x11::Shown {
    let mut last = None;
    eventually("the flyout to settle", || {
        let mut shown = display.shown(app.pid());
        let flyout = (shown.len() == 1).then(|| shown.remove(0))?;
        let bounds = (flyout.x, flyout.y, flyout.width, flyout.height);
        let settled = last == Some(bounds);
        last = Some(bounds);
        sleep(Duration::from_millis(400));
        settled.then_some(flyout)
    })
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_x11_flyout_opens_by_the_panel_in_the_corner_of_the_work_area() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    if session() != "x11" {
        return;
    }
    let display = x11::Display::open();
    let app = App::start();

    let dock = display.dock(false, 33);
    eventually("openbox to leave the dock out", || {
        (display.work_area() == [0, 0, 1920, 1047]).then_some(())
    });
    // xfce4-panel's click on an icon at the bottom right (research.md, A-5).
    app.click(1909, 1064);
    let flyout = x11_flyout(&display, &app);
    assert!(flyout.override_redirect, "{flyout:?}");
    assert_eq!(
        (
            flyout.x + flyout.width,
            flyout.y + flyout.height,
            flyout.width
        ),
        (1920 - MARGIN, 1047 - MARGIN, WIDTH),
        "{flyout:?}"
    );
    assert_eq!(
        display.focus(),
        flyout.id,
        "the flyout has the keyboard (A-1)"
    );

    app.click(1909, 1064);
    eventually("the flyout to close", || {
        display.shown(app.pid()).is_empty().then_some(())
    });
    display.destroy(dock);
    let dock = display.dock(true, 33);
    eventually("openbox to leave the dock out", || {
        (display.work_area() == [0, 33, 1920, 1047]).then_some(())
    });
    sleep(Duration::from_millis(400));
    app.click(1909, 16);
    let flyout = x11_flyout(&display, &app);
    assert_eq!(
        (flyout.x + flyout.width, flyout.y, flyout.width),
        (1920 - MARGIN, 33 + MARGIN, WIDTH),
        "{flyout:?}"
    );
    display.destroy(dock);
}

/// A screenshot of the output, by `grim`: its width, height and RGB pixels.
fn screenshot(folder: &Path) -> (usize, usize, Vec<u8>) {
    let file = folder.join("screen.ppm");
    run("grim", &["-t", "ppm", file.to_str().unwrap()]);
    let bytes = std::fs::read(file).unwrap();
    // P6, width, height, the maximum, each followed by one whitespace.
    let mut fields = Vec::new();
    let mut start = 0;
    while fields.len() < 4 {
        let end = start
            + bytes[start..]
                .iter()
                .position(u8::is_ascii_whitespace)
                .unwrap();
        fields.push(String::from_utf8(bytes[start..end].to_vec()).unwrap());
        start = end + 1;
    }
    let (width, height) = (fields[1].parse().unwrap(), fields[2].parse().unwrap());
    (width, height, bytes[start..].to_vec())
}

/// How much the flyout's shadow darkens the background at most, on any channel.
const SHADOW_DEPTH: u8 = 32;

/// The rows and columns of the output that hold the flyout's panel: what differs from the
/// background, which fills its centre, by more than the shadow around the panel does.
fn drawn(folder: &Path) -> Option<(usize, usize, usize, usize)> {
    let (width, height, pixels) = screenshot(folder);
    let pixel = |x: usize, y: usize| &pixels[(y * width + x) * 3..][..3];
    let background = pixel(width / 2, height / 2).to_vec();
    let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..height {
        for x in 0..width {
            let differs = pixel(x, y)
                .iter()
                .zip(&background)
                .any(|(pixel, background)| pixel.abs_diff(*background) > SHADOW_DEPTH);
            if differs {
                (left, top) = (left.min(x), top.min(y));
                (right, bottom) = (right.max(x), bottom.max(y));
            }
        }
    }
    (left <= right).then_some((left, top, right, bottom))
}

/// Whether the output at `x`, `y` is darker than the background, as the flyout's shadow makes it.
fn shadowed(folder: &Path, x: usize, y: usize) -> bool {
    let (width, height, pixels) = screenshot(folder);
    let pixel = |x: usize, y: usize| &pixels[(y * width + x) * 3..][..3];
    let background = pixel(width / 2, height / 2);
    pixel(x, y)
        .iter()
        .zip(background)
        .all(|(pixel, background)| pixel < background)
}

/// The windows of process `pid` in sway's tree: layer surfaces are not in it.
fn sway_toplevels(pid: u32) -> Vec<String> {
    let tree = run("swaymsg", &["-t", "get_tree", "-r"]);
    let needle = format!("\"pid\": {pid},");
    tree.matches(&needle).map(|_| needle.clone()).collect()
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_wayland_flyout_is_a_layer_surface_anchored_to_the_clicked_corner() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    if session() != "wayland" {
        return;
    }
    let folder = TempDir::new().unwrap();
    let app = App::start();
    // waybar's click on an icon at the top right of a 1920 × 1080 output (research.md, A-5).
    for (x, y, right, bottom) in [(1899, 14, true, false), (20, 1064, false, true)] {
        app.click(x, y);
        let (left, top, right_edge, bottom_edge) = eventually("the flyout to draw", || {
            let first = drawn(folder.path())?;
            sleep(Duration::from_millis(400));
            (drawn(folder.path()) == Some(first)).then_some(first)
        });
        let margin = MARGIN as usize;
        if right {
            assert_eq!(right_edge, 1920 - margin - 1, "click at {x}, {y}");
        } else {
            assert_eq!(left, margin, "click at {x}, {y}");
        }
        if bottom {
            assert_eq!(bottom_edge, 1080 - margin - 1, "click at {x}, {y}");
        } else {
            assert_eq!(top, margin, "click at {x}, {y}");
        }
        assert_eq!(right_edge + 1 - left, WIDTH as usize, "click at {x}, {y}");
        // In the transparent room between the panel and the edge of the output.
        let beside = if right { right_edge + 3 } else { left - 3 };
        assert!(
            shadowed(folder.path(), beside, (top + bottom_edge) / 2),
            "the panel's shadow, click at {x}, {y}"
        );
        assert!(
            sway_toplevels(app.pid()).is_empty(),
            "a layer surface, no window"
        );
        app.click(x, y);
        eventually("the flyout to close", || {
            drawn(folder.path()).is_none().then_some(())
        });
        sleep(Duration::from_millis(400));
    }
}

#[test]
#[ignore = "desktop-harness"]
fn desktop_harness_without_a_tray_host_the_app_opens_an_ordinary_window_and_outlives_it() {
    let _turn = HARNESS_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let no_watcher = NoWatcher::new();
    let mut app = App::start();

    if session() == "x11" {
        let display = x11::Display::open();
        let window = eventually("the window", || display.shown(app.pid()).pop());
        assert!(
            window.managed && !window.override_redirect,
            "an ordinary window: {window:?}"
        );
        // Another window takes the focus: the app's is minimized, not closed.
        let other = display.other_window();
        eventually("the other window to have the focus", || {
            display.give_focus(other);
            sleep(Duration::from_millis(200));
            display.iconic(window.id).then_some(())
        });
        assert!(app.alive(), "the app quit when its window lost the focus");

        // The host comes back while the window is put away: the icon shows the app now, so the
        // window goes, and the app stays.
        drop(no_watcher);
        app.item();
        eventually("the window to close", || {
            (!display.exists(window.id)).then_some(())
        });
        sleep(Duration::from_secs(1));
        assert!(
            app.alive(),
            "the app quit when its window closed with a host"
        );
        app.click(1909, 1064);
        let flyout = x11_flyout(&display, &app);
        assert!(flyout.override_redirect, "the flyout again: {flyout:?}");
        display.destroy(other);
    } else {
        eventually("the window", || {
            (sway_toplevels(app.pid()).len() == 1).then_some(())
        });
        // The host comes back while the window has the focus: it becomes the flyout, whose
        // closing no longer quits the app.
        drop(no_watcher);
        app.item();
        sleep(Duration::from_secs(1));
        app.click(1909, 1064);
        sleep(Duration::from_secs(2));
        assert!(
            app.alive(),
            "the app quit when its window closed with a host"
        );
    }
}
