//! How the app's windows are decorated. On Windows the app draws each window's title bar, which
//! Windows treats as the caption, and Windows draws the frame. On Linux the desktop draws both
//! when it offers to (KDE Plasma, sway, the X11 window managers). Where it asks the app to (GNOME
//! on Wayland), the app draws the title bar and a 1 px border, and a resizable window resizes from
//! its edges.

use gpui::{
    App, CursorStyle, Decorations, Div, Global, Hsla, MouseButton, ParentElement, Pixels,
    ResizeEdge, Stateful, Styled, Tiling, Window, WindowControlArea, div, prelude::*, px,
};

use crate::platform;
use crate::theme::size::TITLEBAR_HEIGHT;

/// Who decorates a window, and so what the app draws of its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    /// Windows: the app draws the title bar, which Windows treats as the caption, and Windows
    /// draws the frame.
    Caption,
    /// Linux, decorated by the desktop: it draws the title bar and the frame, and the app neither.
    Desktop,
    /// Linux, decorated by the app: the title bar, a border on each edge that is not `tiling`'s,
    /// and the edges that resize the window.
    App { tiling: Tiling },
}

/// The chrome every window takes in place of the one its platform reports: the screenshots draw
/// the windows of one platform on another, and the headless platform reports the desktop's.
pub struct ChromeOverride(pub Chrome);

impl Global for ChromeOverride {}

/// How far inside a window's edge a press resizes it.
const RESIZE_ZONE: Pixels = px(6.);

impl Chrome {
    /// The chrome of `window` now: the desktop can switch its decorations while it is open.
    pub fn of(window: &Window, cx: &App) -> Self {
        cx.try_global::<ChromeOverride>().map_or_else(
            || {
                Self::reported(
                    platform::DECORATIONS_AS_REPORTED,
                    window.window_decorations(),
                )
            },
            |chrome| chrome.0,
        )
    }

    /// GPUI reports server-side decorations on Windows too, where the app's title bar is the
    /// caption all the same.
    fn reported(linux: bool, decorations: Decorations) -> Self {
        match (linux, decorations) {
            (false, _) => Self::Caption,
            (true, Decorations::Server) => Self::Desktop,
            (true, Decorations::Client { tiling }) => Self::App { tiling },
        }
    }

    pub fn draws_title_bar(self) -> bool {
        self != Self::Desktop
    }

    /// The height of the title bar the app draws, which what covers the window starts below.
    pub fn title_bar_height(self) -> Pixels {
        if self.draws_title_bar() {
            TITLEBAR_HEIGHT
        } else {
            px(0.)
        }
    }

    /// The edges that resize a resizable window: none that the desktop tiled.
    fn resize_edges(self, resizable: bool) -> Vec<ResizeEdge> {
        let Self::App { tiling } = self else {
            return Vec::new();
        };
        if !resizable {
            return Vec::new();
        }
        let (top, left, right, bottom) = (!tiling.top, !tiling.left, !tiling.right, !tiling.bottom);
        [
            (ResizeEdge::Top, top),
            (ResizeEdge::Bottom, bottom),
            (ResizeEdge::Left, left),
            (ResizeEdge::Right, right),
            (ResizeEdge::TopLeft, top && left),
            (ResizeEdge::TopRight, top && right),
            (ResizeEdge::BottomLeft, bottom && left),
            (ResizeEdge::BottomRight, bottom && right),
        ]
        .into_iter()
        .filter_map(|(edge, free)| free.then_some(edge))
        .collect()
    }
}

/// `root`, which fills `window`, with the frame `chrome` has the app draw: with the app's
/// decorations, a 1 px `border` on each edge that is not tiled, and zones just inside the edges
/// of a resizable window that resize it.
pub fn framed<E: ParentElement + Styled>(
    root: E,
    chrome: Chrome,
    window: &Window,
    border: Hsla,
) -> E {
    let Chrome::App { tiling } = chrome else {
        return root;
    };
    bordered(root.relative(), tiling, border).children(
        chrome
            .resize_edges(window.is_resizable())
            .into_iter()
            .map(resize_zone),
    )
}

/// `element`, the outside of a window the app decorates, with a 1 px `border` on each edge that
/// `tiling` leaves free.
pub fn bordered<E: Styled>(mut element: E, tiling: Tiling, border: Hsla) -> E {
    element = element.border_color(border);
    if !tiling.top {
        element = element.border_t_1();
    }
    if !tiling.bottom {
        element = element.border_b_1();
    }
    if !tiling.left {
        element = element.border_l_1();
    }
    if !tiling.right {
        element = element.border_r_1();
    }
    element
}

/// The zone, just inside the window's `edge`, that resizes it from there.
fn resize_zone(edge: ResizeEdge) -> Div {
    let zone = div()
        .absolute()
        .occlude()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.start_window_resize(edge);
        });
    let z = RESIZE_ZONE;
    match edge {
        ResizeEdge::Top => zone.top_0().left(z).right(z).h(z),
        ResizeEdge::Bottom => zone.bottom_0().left(z).right(z).h(z),
        ResizeEdge::Left => zone.left_0().top(z).bottom(z).w(z),
        ResizeEdge::Right => zone.right_0().top(z).bottom(z).w(z),
        ResizeEdge::TopLeft => zone.top_0().left_0().size(z),
        ResizeEdge::TopRight => zone.top_0().right_0().size(z),
        ResizeEdge::BottomLeft => zone.bottom_0().left_0().size(z),
        ResizeEdge::BottomRight => zone.bottom_0().right_0().size(z),
    }
    .cursor(cursor(edge))
}

fn cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// `title`, the part of a title bar that drags the window: Windows treats it as the caption, and
/// on Linux, where GPUI ignores that, a press hands the move to the desktop.
pub(crate) fn drag_area(title: Stateful<Div>) -> Stateful<Div> {
    let title = title.window_control_area(WindowControlArea::Drag);
    if !platform::MOVE_ON_PRESS {
        return title;
    }
    title.on_mouse_down(MouseButton::Left, |_, window, _| {
        window.start_window_move();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiled(top: bool, left: bool, right: bool, bottom: bool) -> Tiling {
        Tiling {
            top,
            left,
            right,
            bottom,
        }
    }

    #[test]
    fn the_app_draws_the_title_bar_on_windows_and_where_a_linux_desktop_asks_it_to() {
        let client = Decorations::Client {
            tiling: Tiling::default(),
        };
        assert_eq!(
            Chrome::reported(false, Decorations::Server),
            Chrome::Caption
        );
        assert_eq!(Chrome::reported(false, client), Chrome::Caption);
        assert_eq!(Chrome::reported(true, Decorations::Server), Chrome::Desktop);
        assert_eq!(
            Chrome::reported(true, client),
            Chrome::App {
                tiling: Tiling::default()
            }
        );
        assert!(Chrome::Caption.draws_title_bar());
        assert!(!Chrome::Desktop.draws_title_bar());
        assert!(Chrome::reported(true, client).draws_title_bar());
        assert_eq!(Chrome::Desktop.title_bar_height(), px(0.));
        assert_eq!(Chrome::Caption.title_bar_height(), TITLEBAR_HEIGHT);
    }

    #[test]
    fn only_a_resizable_window_the_app_decorates_resizes_from_its_free_edges() {
        let free = Chrome::App {
            tiling: Tiling::default(),
        };
        assert_eq!(free.resize_edges(true).len(), 8);
        assert!(free.resize_edges(false).is_empty());
        assert!(Chrome::Caption.resize_edges(true).is_empty());
        assert!(Chrome::Desktop.resize_edges(true).is_empty());

        // Tiled to the left half of the screen: only the right edge is free to move.
        let left_half = Chrome::App {
            tiling: tiled(true, true, false, true),
        };
        assert_eq!(left_half.resize_edges(true), [ResizeEdge::Right]);
        let maximized = Chrome::App {
            tiling: tiled(true, true, true, true),
        };
        assert!(maximized.resize_edges(true).is_empty());
    }
}
