use std::error::Error;

use bientot_platform::placement::Rect;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as _, InputFocus, Window,
};
use x11rb::rust_connection::RustConnection;

use super::{Layout, Monitor, x11_scale};

type Failure = Box<dyn Error + Send + Sync>;

/// A connection to the X server beside GPUI's, which reads the layout at each click and moves
/// and focuses the flyout's window, since GPUI can do neither (« Spike results »).
pub struct X11 {
    connection: RustConnection,
    screen: usize,
}

impl X11 {
    /// Connects to `display`, `$DISPLAY` when `None`.
    pub fn connect(display: Option<&str>) -> Result<Self, Failure> {
        let (connection, screen) = RustConnection::connect(display)?;
        Ok(Self { connection, screen })
    }

    /// The X screen, which GPUI names its display by.
    pub fn screen(&self) -> u64 {
        self.screen as u64
    }

    /// The scale GPUI draws at, see [`x11_scale`].
    pub fn scale(&self) -> f32 {
        let xft_dpi = x11rb::resource_manager::new_from_default(&self.connection)
            .ok()
            .and_then(|database| {
                database
                    .get_value::<f32>("Xft.dpi", "Xft.dpi")
                    .ok()
                    .flatten()
            });
        x11_scale(
            std::env::var("GPUI_X11_SCALE_FACTOR").ok().as_deref(),
            xft_dpi,
        )
    }

    /// Whether a compositing manager runs on the screen: EWMH has it own the selection
    /// `_NET_WM_CM_S<screen>`. Without one, the X server draws a transparent window over black.
    pub fn compositing(&self) -> Result<bool, Failure> {
        let name = format!("_NET_WM_CM_S{}", self.screen);
        let atom = self
            .connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom;
        let owner = self.connection.get_selection_owner(atom)?.reply()?.owner;
        Ok(owner != x11rb::NONE)
    }

    pub fn layout(&self) -> Result<Layout, Failure> {
        read(&self.connection, self.screen)
    }

    /// Puts the top left corner of `window` at `x`, `y` of the root window. GPUI opens a pop-up
    /// 2 px right of where it is asked, and cannot move a window it opened.
    pub fn move_window(&self, window: Window, x: i32, y: i32) -> Result<(), Failure> {
        self.connection
            .configure_window(window, &ConfigureWindowAux::new().x(x).y(y))?
            .check()?;
        Ok(())
    }

    /// Gives `window` the keyboard when it is override-redirect, as GPUI's pop-ups are: no window
    /// manager focuses such a window, and without the focus it never hears of Esc nor of a click
    /// elsewhere (A-1).
    pub fn focus(&self, window: Window) -> Result<(), Failure> {
        let attributes = self.connection.get_window_attributes(window)?.reply()?;
        if attributes.override_redirect {
            self.connection
                .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?
                .check()?;
        }
        Ok(())
    }
}

fn read(connection: &RustConnection, screen: usize) -> Result<Layout, Failure> {
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
    let current_desktop = cardinals(connection, root, b"_NET_CURRENT_DESKTOP")?
        .first()
        .copied()
        .unwrap_or(0);
    let net_workarea = cardinals(connection, root, b"_NET_WORKAREA")?
        .chunks_exact(4)
        .nth(current_desktop as usize)
        .map(|area| Rect {
            x: area[0] as i32,
            y: area[1] as i32,
            width: area[2] as i32,
            height: area[3] as i32,
        });
    Ok(Layout {
        monitors,
        net_workarea,
    })
}

/// The `CARDINAL` values of the property `name` of `window`; none when it is not set.
fn cardinals(
    connection: &RustConnection,
    window: Window,
    name: &[u8],
) -> Result<Vec<u32>, Failure> {
    let atom = connection.intern_atom(true, name)?.reply()?.atom;
    if atom == x11rb::NONE {
        return Ok(Vec::new());
    }
    let reply = connection
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, u32::MAX)?
        .reply()?;
    Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
}
