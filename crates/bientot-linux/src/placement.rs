//! Where the flyout opens on an X11 desktop: the monitor under the click, its work area, and the
//! corner of that work area nearest the panel (`contracts/linux-backends.md`, « Flyout
//! placement »).
//!
//! An X11 window manager publishes one `_NET_WORKAREA` rectangle per virtual desktop, spanning
//! every monitor, and RandR names the monitors. The work area of one monitor is the first clipped
//! to the second: exact on one monitor, and an approximation on several (assumption A-2).

#[cfg(target_os = "linux")]
mod native;

#[cfg(target_os = "linux")]
pub use native::{X11Desktop, read_x11_desktop};

use bientot_platform::placement::{Corner, Rect};
use bientot_platform::tray::ScreenPoint;

/// A monitor as RandR names it, in physical pixels of the root window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub bounds: Rect,
    pub primary: bool,
}

/// The monitor under `click`, else the primary monitor, else the first one.
pub fn clicked_monitor(monitors: &[Monitor], click: Option<ScreenPoint>) -> Option<Monitor> {
    let under_click = click.and_then(|point| {
        monitors
            .iter()
            .find(|monitor| contains(monitor.bounds, point))
    });
    under_click
        .or_else(|| monitors.iter().find(|monitor| monitor.primary))
        .or_else(|| monitors.first())
        .copied()
}

/// The part of `monitor` inside the window manager's work area, or the whole monitor when the
/// window manager publishes none or its work area misses the monitor.
pub fn work_area(monitor: Rect, net_workarea: Option<Rect>) -> Rect {
    net_workarea
        .and_then(|area| intersection(monitor, area))
        .unwrap_or(monitor)
}

/// The corner of `area` nearest `click`, where the panel that holds the tray icon is; top right
/// when the click is unknown.
pub fn nearest_corner(area: Rect, click: Option<ScreenPoint>) -> Corner {
    let Some(point) = click else {
        return Corner::TopRight;
    };
    let left = 2 * point.x < 2 * area.x + area.width;
    let top = 2 * point.y < 2 * area.y + area.height;
    match (top, left) {
        (true, true) => Corner::TopLeft,
        (true, false) => Corner::TopRight,
        (false, true) => Corner::BottomLeft,
        (false, false) => Corner::BottomRight,
    }
}

fn contains(rect: Rect, point: ScreenPoint) -> bool {
    (rect.x..rect.x + rect.width).contains(&point.x)
        && (rect.y..rect.y + rect.height).contains(&point.y)
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (left < right && top < bottom).then(|| Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests;
