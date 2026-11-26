//! Where the flyout opens on a Linux desktop (`contracts/linux-backends.md`, « Flyout
//! placement »): in a corner of the clicked monitor's work area on X11, in a corner of the
//! output as a layer-shell surface on Wayland compositors that offer one, where the compositor
//! puts it elsewhere, and in an ordinary window when no tray host shows the icon.
//!
//! An X11 window manager publishes one `_NET_WORKAREA` rectangle per virtual desktop, spanning
//! every monitor, and RandR names the monitors. The work area of one monitor is the first clipped
//! to the second: exact on one monitor, and an approximation on several (assumption A-2).

#[cfg(target_os = "linux")]
mod native;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
pub use native::LinuxDesktop;
#[cfg(target_os = "linux")]
pub use x11::X11;

use bientot_platform::placement::{Corner, FlyoutPlacement, Rect, WorkArea};
use bientot_platform::tray::ScreenPoint;

/// The space between the flyout and the edges of the area it opens in. The app keeps it on X11,
/// where it computes the bounds, and the compositor on Wayland, where the flyout is anchored.
pub const MARGIN: i32 = 12;

/// The height kept off a Wayland output for a panel, which no client can measure there.
pub const UNMEASURED_PANEL: i32 = 48;

/// A monitor as RandR names it, in physical pixels of the root window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub bounds: Rect,
    pub primary: bool,
}

/// What an X11 desktop says about its monitors and the area its panels leave free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// From RandR's `GetMonitors`, or the root window when RandR names none.
    pub monitors: Vec<Monitor>,
    /// `_NET_WORKAREA` of the current desktop, when the window manager publishes it.
    pub net_workarea: Option<Rect>,
}

impl Layout {
    /// The work area of the monitor under `click`, with that monitor.
    pub fn work_area_at(&self, click: Option<ScreenPoint>) -> Option<(Monitor, Rect)> {
        let monitor = clicked_monitor(&self.monitors, click)?;
        Some((monitor, work_area(monitor.bounds, self.net_workarea)))
    }

    /// The flyout in the corner of the clicked monitor's work area nearest the click. GPUI names
    /// the X screen `screen` and draws every window at one `scale`.
    pub fn placement(
        &self,
        click: Option<ScreenPoint>,
        screen: u64,
        scale: f32,
    ) -> Option<FlyoutPlacement> {
        let (_, bounds) = self.work_area_at(click)?;
        Some(FlyoutPlacement::At {
            work_area: WorkArea {
                display: screen,
                bounds,
                scale,
            },
            corner: nearest_corner(bounds, click),
        })
    }
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

/// The scale GPUI draws X11 windows at: `GPUI_X11_SCALE_FACTOR` when it is a number, else the
/// resources' `Xft.dpi` over 96, else 1. GPUI then tries RandR's physical size, which this
/// leaves out: the flyout's first frame moves it where that scale puts it.
pub fn x11_scale(variable: Option<&str>, xft_dpi: Option<f32>) -> f32 {
    let valid = |scale: &f32| scale.is_normal() && *scale > 0.;
    variable
        .and_then(|value| value.parse().ok())
        .filter(valid)
        .or_else(|| xft_dpi.map(|dpi| dpi / 96.).filter(valid))
        .unwrap_or(1.)
}

/// The desktops `XDG_CURRENT_DESKTOP` names, lower-cased.
pub fn desktops(xdg_current_desktop: &str) -> Vec<String> {
    xdg_current_desktop
        .split(':')
        .filter(|name| !name.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A Wayland output, as `xdg_output` and `wl_output` describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    /// Its place in the compositor's layout, in logical pixels.
    pub logical: Rect,
    /// Its height in pixels of its current mode, once rotated as it is shown.
    pub physical_height: i32,
}

/// What a Wayland session lets the flyout be, read once at start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaylandShell {
    /// Whether the compositor offers `zwlr_layer_shell_v1`.
    pub layer_shell: bool,
    pub outputs: Vec<Output>,
}

impl WaylandShell {
    /// A layer-shell surface in the corner the click is nearest on every output it can be on, or
    /// the desktop's default corner; without layer-shell, where the compositor puts it (A-4).
    pub fn placement(&self, click: Option<ScreenPoint>, desktops: &[String]) -> FlyoutPlacement {
        if !self.layer_shell {
            return FlyoutPlacement::CompositorPlaced;
        }
        FlyoutPlacement::Layer {
            corner: layer_corner(click, &self.outputs, desktops),
            margin: MARGIN,
        }
    }

    /// The height the flyout may fill, in physical pixels: the shortest output's, less a panel of
    /// [`UNMEASURED_PANEL`] logical pixels, since the compositor chooses the output.
    pub fn usable_height(&self) -> Option<i32> {
        self.outputs
            .iter()
            .filter(|output| output.logical.height > UNMEASURED_PANEL)
            .map(|output| {
                let free = i64::from(output.logical.height - UNMEASURED_PANEL);
                let height =
                    i64::from(output.physical_height) * free / i64::from(output.logical.height);
                height as i32
            })
            .min()
    }
}

/// The corner of the output a click on the tray icon is nearest. Hosts give the position on the
/// output, not on the layout (waybar, « Spike results »), so the click may be on any output large
/// enough to hold it: when they all agree, that corner, else `default_corner`.
pub fn layer_corner(click: Option<ScreenPoint>, outputs: &[Output], desktops: &[String]) -> Corner {
    let default = default_corner(desktops);
    let Some(point) = click else {
        return default;
    };
    let mut corners = outputs
        .iter()
        .map(|output| Rect {
            x: 0,
            y: 0,
            ..output.logical
        })
        .filter(|area| contains(*area, point))
        .map(|area| nearest_corner(area, click));
    match corners.next() {
        Some(corner) if corners.all(|other| other == corner) => corner,
        _ => default,
    }
}

/// Where the flyout goes when the click tells nothing: Plasma's panel is at the bottom, the
/// others' at the top.
pub fn default_corner(desktops: &[String]) -> Corner {
    if desktops.iter().any(|name| name == "kde") {
        Corner::BottomRight
    } else {
        Corner::TopRight
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
