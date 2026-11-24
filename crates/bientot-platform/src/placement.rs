//! Where the flyout opens, in plain integers: the platform answers, and the app maps the answer
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlyoutPlacement {
    /// The app chooses the bounds (Windows, X11).
    At { bounds: Rect },
    /// A layer-shell surface anchored to a corner (KDE, wlroots).
    Layer { corner: Corner, margin: i32 },
    /// The compositor chooses (GNOME on Wayland).
    CompositorPlaced,
    /// An ordinary, minimizable window: nothing shows the tray icon.
    Window,
}

/// The desktop the flyout opens on.
pub trait Desktop {
    /// Where a flyout of `content` size opens after a click at `click` on the tray icon.
    fn flyout_placement(
        &self,
        content: (i32, i32),
        click: Option<ScreenPoint>,
        tray: TrayHost,
    ) -> FlyoutPlacement;

    /// The height the flyout may use, to cap it and scroll its body.
    fn usable_height(&self, click: Option<ScreenPoint>) -> Option<i32>;

    /// Whether the system rounds and shadows the window, or the app draws both.
    fn system_draws_frame(&self) -> bool;
}

/// A desktop that gives the same answers whatever it is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InMemoryDesktop {
    pub placement: FlyoutPlacement,
    pub usable_height: Option<i32>,
    pub system_draws_frame: bool,
}

impl Desktop for InMemoryDesktop {
    fn flyout_placement(
        &self,
        _content: (i32, i32),
        _click: Option<ScreenPoint>,
        _tray: TrayHost,
    ) -> FlyoutPlacement {
        self.placement
    }

    fn usable_height(&self, _click: Option<ScreenPoint>) -> Option<i32> {
        self.usable_height
    }

    fn system_draws_frame(&self) -> bool {
        self.system_draws_frame
    }
}
