//! Where the flyout opens, in plain numbers: the platform answers, and the app maps the answer
//! to its window options.

use crate::tray::{ScreenPoint, TrayHost};

/// A rectangle of the desktop, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The part of a monitor no taskbar or panel covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkArea {
    /// The system's id of the monitor, which GPUI names its display by.
    pub display: u64,
    /// In physical pixels.
    pub bounds: Rect,
    /// Physical pixels per logical pixel on that monitor.
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlyoutPlacement {
    /// The app chooses the bounds, in a corner of the work area (Windows, X11).
    At { work_area: WorkArea },
    /// A layer-shell surface anchored to a corner (KDE, wlroots).
    Layer { corner: Corner, margin: i32 },
    /// The compositor chooses (GNOME on Wayland).
    CompositorPlaced,
    /// An ordinary, minimizable window: nothing shows the tray icon.
    Window,
}

/// The desktop the flyout opens on.
pub trait Desktop {
    /// Where the flyout opens after a click at `click` on the tray icon.
    fn flyout_placement(&self, click: Option<ScreenPoint>, tray: TrayHost) -> FlyoutPlacement;

    /// The height of the area the flyout opens in, in physical pixels: the app keeps the panel a
    /// margin from both edges, and scrolls its body past that.
    fn usable_height(&self, click: Option<ScreenPoint>) -> Option<i32>;

    /// Whether the system rounds and shadows the window, or the app draws both.
    fn system_draws_frame(&self) -> bool;
}

/// A desktop that gives the same answers whatever it is asked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InMemoryDesktop {
    pub placement: FlyoutPlacement,
    pub usable_height: Option<i32>,
    pub system_draws_frame: bool,
}

impl Desktop for InMemoryDesktop {
    fn flyout_placement(&self, _click: Option<ScreenPoint>, _tray: TrayHost) -> FlyoutPlacement {
        self.placement
    }

    fn usable_height(&self, _click: Option<ScreenPoint>) -> Option<i32> {
        self.usable_height
    }

    fn system_draws_frame(&self) -> bool {
        self.system_draws_frame
    }
}
