use std::error::Error;

use bientot_platform::placement::Rect;
use bientot_platform::tray::ScreenPoint;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::{Monitor, clicked_monitor, work_area};

/// What an X11 desktop says about its monitors and the area its panels leave free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct X11Desktop {
    /// From RandR's `GetMonitors`, or the root window when RandR names none.
    pub monitors: Vec<Monitor>,
    /// `_NET_WORKAREA` of the current desktop, when the window manager publishes it.
    pub net_workarea: Option<Rect>,
}

impl X11Desktop {
    /// The work area of the monitor under `click`, with that monitor.
    pub fn work_area_at(&self, click: Option<ScreenPoint>) -> Option<(Monitor, Rect)> {
        let monitor = clicked_monitor(&self.monitors, click)?;
        Some((monitor, work_area(monitor.bounds, self.net_workarea)))
    }
}

/// Reads the monitors and the work area of the X11 display `display`, `$DISPLAY` when `None`, on
/// a connection of its own.
pub fn read_x11_desktop(display: Option<&str>) -> Result<X11Desktop, Box<dyn Error + Send + Sync>> {
    let (connection, screen) = RustConnection::connect(display)?;
    let root_screen = &connection.setup().roots[screen];
    let root = root_screen.root;
    // A server without RandR 1.5, as some VNC and XRDP servers are, falls back to the root screen.
    let randr_monitors = connection
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.monitors)
        .unwrap_or_default();
    let mut monitors: Vec<Monitor> = randr_monitors
        .iter()
        .map(|monitor| Monitor {
            bounds: Rect {
                x: monitor.x.into(),
                y: monitor.y.into(),
                width: monitor.width.into(),
                height: monitor.height.into(),
            },
            primary: monitor.primary,
        })
        .collect();
    if monitors.is_empty() {
        monitors.push(Monitor {
            bounds: Rect {
                x: 0,
                y: 0,
                width: root_screen.width_in_pixels.into(),
                height: root_screen.height_in_pixels.into(),
            },
            primary: true,
        });
    }
    let current_desktop = cardinals(&connection, root, b"_NET_CURRENT_DESKTOP")?
        .first()
        .copied()
        .unwrap_or(0);
    let net_workarea = cardinals(&connection, root, b"_NET_WORKAREA")?
        .chunks_exact(4)
        .nth(current_desktop as usize)
        .map(|area| Rect {
            x: area[0] as i32,
            y: area[1] as i32,
            width: area[2] as i32,
            height: area[3] as i32,
        });
    Ok(X11Desktop {
        monitors,
        net_workarea,
    })
}

/// The `CARDINAL` values of the property `name` of `window`; none when it is not set.
fn cardinals(
    connection: &RustConnection,
    window: Window,
    name: &[u8],
) -> Result<Vec<u32>, Box<dyn Error + Send + Sync>> {
    let atom = connection.intern_atom(true, name)?.reply()?.atom;
    if atom == x11rb::NONE {
        return Ok(Vec::new());
    }
    let reply = connection
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, u32::MAX)?
        .reply()?;
    Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
}
