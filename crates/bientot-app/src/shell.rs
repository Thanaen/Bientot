//! Opens and closes the flyout, and fits its window to its content and to the work area.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bientot_platform::placement::{Desktop, FlyoutPlacement, Rect};
use bientot_platform::tray::{ScreenPoint, TrayHost};
use gpui::{
    App, AppContext as _, Bounds, DevicePixels, DisplayId, Global, Pixels, Size, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions, point, px, size,
};

use crate::flyout::Flyout;
use crate::host::Today;
use crate::live::Tracking;
use crate::widgets::Notice;
use crate::workday::DayView;
use crate::{platform, theme};

/// Gap between the flyout's panel and the taskbar or the screen edges, in physical pixels, as
/// in the Windows 11 flyouts.
const MARGIN: i32 = 12;

/// A click on the tray icon first takes the focus away from an open flyout, which closes it.
/// Without this delay the same click would open it again at once.
const REOPEN_DELAY: Duration = Duration::from_millis(300);

/// The size of the flyout's panel when its window was last fitted: the size it most likely has
/// at the next opening.
static LAST_PANEL: Mutex<Option<Size<Pixels>>> = Mutex::new(None);

#[derive(Default)]
struct FlyoutState {
    window: Option<WindowHandle<Flyout>>,
    /// The flyout the window shows, which an action pressed in it updates while the window is
    /// being updated: no other update of the window can enter then.
    view: Option<WeakEntity<Flyout>>,
    closed_at: Option<Instant>,
}

impl Global for FlyoutState {}

/// Where the flyout opens: the desktop, whether a tray shows the icon, and where the icon was
/// last clicked.
struct Placement {
    desktop: Rc<dyn Desktop>,
    tray: TrayHost,
    click: Option<ScreenPoint>,
}

impl Global for Placement {}

/// Opens the flyouts on `desktop`, from a tray that `tray` says is shown or not.
pub fn install(desktop: Rc<dyn Desktop>, tray: TrayHost, cx: &mut App) {
    cx.set_global(Placement {
        desktop,
        tray,
        click: None,
    });
}

/// A click on the tray icon, at `at` when the tray tells: opens the flyout on that monitor, or
/// closes it.
pub fn tray_clicked(at: Option<ScreenPoint>, cx: &mut App) {
    cx.global_mut::<Placement>().click = at;
    toggle_flyout(cx);
}

/// The next flyout opens for a tray that shows the icon or not.
pub fn tray_changed(tray: TrayHost, cx: &mut App) {
    cx.global_mut::<Placement>().tray = tray;
}

/// Where the flyout opens now. Without a desktop, as in the screenshots, where the system puts
/// it.
fn placement(cx: &App) -> FlyoutPlacement {
    cx.try_global::<Placement>()
        .map_or(FlyoutPlacement::CompositorPlaced, |placement| {
            placement
                .desktop
                .flyout_placement(placement.click, placement.tray)
        })
}

/// Whether the system rounds and shadows the flyout's window, or the app draws both. Without a
/// desktop, as in the screenshots, the app does.
pub fn system_draws_frame(cx: &App) -> bool {
    cx.try_global::<Placement>()
        .is_some_and(|placement| placement.desktop.system_draws_frame())
}

pub fn toggle_flyout(cx: &mut App) {
    let state = cx.default_global::<FlyoutState>();
    if let Some(window) = state.window.take() {
        state.closed_at = Some(Instant::now());
        window
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        return;
    }
    if state
        .closed_at
        .is_some_and(|closed| closed.elapsed() < REOPEN_DELAY)
    {
        return;
    }
    open_flyout(cx);
}

/// Opens the flyout on the current day, or brings an open one to the front.
pub fn open_flyout(cx: &mut App) {
    if let Some(window) = cx.default_global::<FlyoutState>().window {
        window
            .update(cx, |_, window, _| window.activate_window())
            .ok();
        return;
    }
    open_new_flyout(None, cx);
}

/// Opens the flyout, or brings it to the front, with `notice` above the day. A flyout it opens
/// shows `notice` from its first frame, at rest; in an open one `notice` comes in.
pub fn open_flyout_with(notice: Notice, cx: &mut App) {
    if cx.default_global::<FlyoutState>().window.is_none() {
        open_new_flyout(Some(notice), cx);
        return;
    }
    open_flyout(cx);
    update_flyout(cx, |flyout, cx| flyout.set_notice(notice, cx));
}

/// `App::open_window` draws the first frame before it returns, so what shows at opening is
/// given here rather than set on the window afterwards.
fn open_new_flyout(notice: Option<Notice>, cx: &mut App) {
    // Through the host, so that the tray menu, built ahead of its opening, offers the commands of
    // the caught-up day too.
    crate::host::catch_up(cx);
    let today = cx.global::<Today>();
    let (day, tracking, alerts) = (
        today.view().clone(),
        today.tracking(),
        today.opening_alerts(),
    );
    let options = window_options(placement(cx), last_panel());
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| Flyout::new(day, tracking, notice, window, cx).with_alerts(alerts))
    }) {
        Ok(window) => {
            // `focus: true` only shows the window activated, and Windows keeps a window of a
            // background process out of the foreground: the flyout then never learns that a
            // click went elsewhere, and stays open. `activate_window` forces it to the front.
            window
                .update(cx, |_, window, _| window.activate_window())
                .ok();
            cx.global_mut::<Today>().announced();
            let view = window.entity(cx).ok().map(|view| view.downgrade());
            let state = cx.default_global::<FlyoutState>();
            state.window = Some(window);
            state.view = view;
        }
        Err(error) => log::error!("cannot open the flyout: {error:#}"),
    }
}

/// The window the flyout opens in at `placement`, its panel `panel` large. Windows shows the
/// window as soon as GPUI creates it, empty until its first frame fits it: opened elsewhere than
/// where that frame puts it, it would flash there, at the top left of the primary monitor. So in a
/// work area it opens in the corner the frame will fit it in, at the size its panel last had.
fn window_options(placement: FlyoutPlacement, panel: Size<Pixels>) -> WindowOptions {
    let popup = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            panel,
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        // Opaque, the default: on Windows, `Transparent` has Windows compose a frosted layer
        // behind the whole window. The window is the panel, and Windows draws its shadow.
        ..Default::default()
    };
    match placement {
        FlyoutPlacement::At { work_area } => {
            let client = flyout_bounds(bounds(work_area.bounds), panel, work_area.scale);
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(logical_bounds(
                    client,
                    work_area.scale,
                ))),
                display_id: Some(DisplayId::new(work_area.display)),
                ..popup
            }
        }
        // GPUI has `WindowKind::LayerShell` only when built for Wayland, and no backend answers
        // `Layer` yet.
        FlyoutPlacement::Layer { .. } | FlyoutPlacement::CompositorPlaced => popup,
        FlyoutPlacement::Window => WindowOptions {
            titlebar: Some(TitlebarOptions::default()),
            kind: WindowKind::Normal,
            is_movable: true,
            is_minimizable: true,
            ..popup
        },
    }
}

/// `rect` in GPUI's bounds.
fn bounds(rect: Rect) -> Bounds<i32> {
    Bounds::new(point(rect.x, rect.y), size(rect.width, rect.height))
}

/// The panel's size at the last fitting, or a guess before the first one.
fn last_panel() -> Size<Pixels> {
    LAST_PANEL
        .lock()
        .unwrap()
        .unwrap_or(size(theme::size::FLYOUT_WIDTH, px(400.)))
}

/// `physical` at `scale`, in the logical pixels GPUI takes window bounds in. GPUI rounds them
/// back to the same physical pixels.
pub fn logical_bounds(physical: Bounds<i32>, scale: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(
            DevicePixels(physical.origin.x),
            DevicePixels(physical.origin.y),
        ),
        size(
            DevicePixels(physical.size.width),
            DevicePixels(physical.size.height),
        ),
    )
    .to_pixels(scale)
}

/// Shows what « Utiliser la détection intégrée » answered in the open flyout, if any.
pub fn show_switch(failure: Option<Notice>, cx: &mut App) {
    update_flyout(cx, |flyout, cx| flyout.switched(failure, cx));
}

/// Shows a new state of the day in the open flyout, if any.
pub fn show_day(
    day: DayView,
    tracking: Tracking,
    recalculated: bool,
    refresh_failure: Option<Notice>,
    cx: &mut App,
) {
    update_flyout(cx, |flyout, cx| {
        flyout.show(day, tracking, recalculated, refresh_failure, cx)
    });
}

/// Runs `update` on the open flyout, if any. Through its view rather than its window: an action
/// pressed in the flyout reaches the day while its window is being updated.
fn update_flyout(cx: &mut App, update: impl FnOnce(&mut Flyout, &mut gpui::Context<Flyout>)) {
    let state = cx.default_global::<FlyoutState>();
    if let (Some(_), Some(view)) = (state.window, state.view.clone()) {
        view.update(cx, update).ok();
    }
}

/// Called when the flyout closed itself (focus lost, Esc), so the next tray click reopens it.
pub fn flyout_closed(cx: &mut App) {
    let state = cx.default_global::<FlyoutState>();
    state.window = None;
    state.view = None;
    state.closed_at = Some(Instant::now());
}

/// What the flyout's window was last fitted to: its content's logical size, at the window's
/// scale. The same size spans other physical pixels at another scale, and after a DPI change
/// Windows moves and resizes the window by its own rule, so a new scale fits it again too.
#[derive(Clone, Default)]
pub struct Fitted(Rc<Cell<Option<Fit>>>);

#[derive(Clone, Copy, PartialEq)]
struct Fit {
    content: Size<Pixels>,
    scale: f32,
}

impl Fitted {
    /// Records that the window shows `content` at `scale`, and returns whether it must be fitted
    /// again with [`fit_window`]: it was last fitted to something else, or never.
    pub fn refit(&self, content: Size<Pixels>, scale: f32) -> bool {
        let fit = Some(Fit { content, scale });
        self.0.replace(fit) != fit
    }
}

/// Gives the window the measured content size and, in a work area, puts it in the corner next
/// to the taskbar, like the Windows 11 flyouts: see [`flyout_bounds`].
pub fn fit_window(window: &mut Window, content: Size<Pixels>, cx: &App) {
    *LAST_PANEL.lock().unwrap() = Some(content);
    match placement(cx) {
        FlyoutPlacement::At { work_area } => {
            let client = flyout_bounds(bounds(work_area.bounds), content, window.scale_factor());
            platform::place_window(window, client);
        }
        FlyoutPlacement::Layer { .. }
        | FlyoutPlacement::CompositorPlaced
        | FlyoutPlacement::Window => window.resize(content),
    }
}

/// The height of the work area the flyout opens in, in physical pixels: its panel is no taller
/// than [`panel_max_height`] allows, and its body scrolls past it. Read for each frame, since a
/// new scale also resizes the taskbar. `None` when the desktop does not tell it.
pub fn work_area_height(cx: &App) -> Option<i32> {
    let placement = cx.try_global::<Placement>()?;
    placement.desktop.usable_height(placement.click)
}

/// The tallest the flyout's panel can be in a work area `height` physical pixels tall at
/// `scale`, MARGIN from both its edges.
pub fn panel_max_height(height: i32, scale: f32) -> Pixels {
    px((height - 2 * MARGIN).max(0) as f32 / scale)
}

/// The client rectangle the panel of `panel` logical size fills in `work_area`, in physical
/// pixels at `scale`: in its bottom right corner, MARGIN from the edges, and no higher than
/// MARGIN below its top.
pub fn flyout_bounds(work_area: Bounds<i32>, panel: Size<Pixels>, scale: f32) -> Bounds<i32> {
    let width = (f32::from(panel.width) * scale).round() as i32;
    let tallest = work_area.size.height - 2 * MARGIN;
    let height = ((f32::from(panel.height) * scale).round() as i32).min(tallest);
    Bounds::new(
        point(
            work_area.right() - width - MARGIN,
            work_area.bottom() - height - MARGIN,
        ),
        size(width, height),
    )
}

/// How far each edge of a window lies outside its client rectangle, in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameInsets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The window rectangle whose client rectangle is `client` once `frame` surrounds it.
pub fn window_bounds(client: Bounds<i32>, frame: FrameInsets) -> Bounds<i32> {
    Bounds::from_corners(
        point(client.left() - frame.left, client.top() - frame.top),
        point(client.right() + frame.right, client.bottom() + frame.bottom),
    )
}

#[cfg(test)]
mod tests {
    use bientot_platform::placement::WorkArea;

    use super::*;

    fn area(left: i32, top: i32, right: i32, bottom: i32) -> Bounds<i32> {
        Bounds::from_corners(point(left, top), point(right, bottom))
    }

    /// The flyout panel, `height` logical pixels tall.
    fn panel(height: f32) -> Size<Pixels> {
        size(theme::size::FLYOUT_WIDTH, px(height))
    }

    #[test]
    fn a_flyout_that_fits_sits_in_the_corner_above_the_taskbar() {
        assert_eq!(
            flyout_bounds(area(0, 0, 1920, 1032), panel(600.), 1.),
            Bounds::new(point(1548, 420), size(360, 600)),
        );
    }

    #[test]
    fn a_flyout_taller_than_a_short_work_area_keeps_its_panel_a_margin_below_the_top() {
        // 1366 × 768 at 125 %, above a taskbar of 60 physical pixels.
        let bounds = flyout_bounds(area(0, 0, 1366, 708), panel(700.), 1.25);
        assert_eq!(bounds, Bounds::new(point(904, 12), size(450, 684)));
        assert_eq!(bounds.bottom(), 708 - 12);
    }

    #[test]
    fn a_panel_as_tall_as_a_short_work_area_allows_fills_it() {
        let tallest = panel_max_height(708, 1.25);
        assert_eq!(tallest, px(547.2));
        assert_eq!(
            flyout_bounds(area(0, 0, 1366, 708), panel(f32::from(tallest)), 1.25),
            Bounds::new(point(904, 12), size(450, 684)),
        );
    }

    #[test]
    fn a_new_scale_fits_a_flyout_of_the_same_size_again() {
        let fitted = Fitted::default();
        assert!(fitted.refit(panel(600.), 1.), "the first frame");
        assert!(
            !fitted.refit(panel(600.), 1.),
            "a frame that changes nothing"
        );
        assert!(fitted.refit(panel(600.), 1.5), "after a DPI change");
        assert!(!fitted.refit(panel(600.), 1.5));
        assert!(fitted.refit(panel(640.), 1.5), "a taller content");
    }

    #[test]
    fn a_taskbar_at_the_top_or_on_the_left_leaves_the_flyout_in_the_bottom_right_corner() {
        assert_eq!(
            flyout_bounds(area(0, 48, 1920, 1080), panel(2000.), 1.),
            Bounds::new(point(1548, 60), size(360, 1008)),
        );
        assert_eq!(
            flyout_bounds(area(48, 0, 1920, 1080), panel(600.), 1.),
            Bounds::new(point(1548, 468), size(360, 600)),
        );
    }

    #[test]
    fn the_flyout_opens_on_the_physical_pixels_its_first_frame_fits_it_to() {
        // 1366 × 768 at 125 %, and a monitor at 150 % left of the primary one.
        for (work_area, scale) in [
            (area(0, 0, 1366, 708), 1.25),
            (area(-2560, 0, 0, 1392), 1.5),
            (area(0, 0, 1920, 1032), 1.),
        ] {
            let fitted = flyout_bounds(work_area, panel(613.), scale);
            let opening = logical_bounds(fitted, scale).to_device_pixels(scale);
            assert_eq!(
                Bounds::new(
                    point(opening.origin.x.0, opening.origin.y.0),
                    size(opening.size.width.0, opening.size.height.0),
                ),
                fitted,
                "at {scale}",
            );
        }
    }

    #[test]
    fn the_window_extends_past_the_client_rectangle_by_its_frame() {
        // No top edge: each inset must move its own edge only.
        let frame = FrameInsets {
            left: 7,
            top: 0,
            right: 7,
            bottom: 7,
        };
        assert_eq!(
            window_bounds(Bounds::new(point(1548, 420), size(360, 600)), frame),
            Bounds::new(point(1541, 420), size(374, 607)),
        );
    }

    #[test]
    fn a_window_on_a_monitor_left_of_the_primary_keeps_its_negative_origin() {
        let frame = FrameInsets {
            left: 8,
            top: 31,
            right: 8,
            bottom: 8,
        };
        assert_eq!(
            window_bounds(Bounds::new(point(-372, 420), size(360, 600)), frame),
            Bounds::new(point(-380, 389), size(376, 639)),
        );
    }

    #[test]
    fn a_flyout_in_a_work_area_opens_on_its_monitor_where_its_first_frame_fits_it() {
        // A monitor at 150 % left of the primary one.
        let work_area = WorkArea {
            display: 7,
            bounds: Rect {
                x: -2560,
                y: 0,
                width: 2560,
                height: 1392,
            },
            scale: 1.5,
        };
        let options = window_options(FlyoutPlacement::At { work_area }, panel(613.));
        assert_eq!(options.display_id, Some(DisplayId::new(7)));
        assert_eq!(options.kind, WindowKind::PopUp);
        let Some(WindowBounds::Windowed(bounds)) = options.window_bounds else {
            panic!("a windowed flyout");
        };
        let opening = bounds.to_device_pixels(1.5);
        assert_eq!(
            Bounds::new(
                point(opening.origin.x.0, opening.origin.y.0),
                size(opening.size.width.0, opening.size.height.0),
            ),
            Bounds::new(point(-552, 460), size(540, 920)),
        );
    }

    #[test]
    fn a_flyout_the_system_places_opens_at_its_panel_size() {
        let options = window_options(FlyoutPlacement::CompositorPlaced, panel(613.));
        assert_eq!(options.display_id, None);
        assert_eq!(options.kind, WindowKind::PopUp);
        assert_eq!(
            options.window_bounds,
            Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                panel(613.)
            ))),
        );
    }

    #[test]
    fn without_a_tray_the_flyout_opens_in_an_ordinary_window() {
        let options = window_options(FlyoutPlacement::Window, panel(613.));
        assert_eq!(options.kind, WindowKind::Normal);
        assert!(options.is_minimizable);
        assert!(options.titlebar.is_some());
    }

    #[test]
    fn without_a_frame_the_window_is_the_client_rectangle() {
        let client = Bounds::new(point(1548, 420), size(360, 600));
        assert_eq!(window_bounds(client, FrameInsets::default()), client);
    }
}
