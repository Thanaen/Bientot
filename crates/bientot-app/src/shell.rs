//! Opens and closes the flyout, and fits its window to its content and to the work area.
//!
//! Without a tray host the flyout is an ordinary window (window mode, FR-012): it minimizes where
//! a flyout closes, so that the app keeps running, and the shell switches between the two as the
//! host comes and goes.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bientot_core::diagnostics::DiagnosticLogLevel;
use bientot_platform::placement::{Corner, Desktop, FlyoutPlacement, Frame, Rect};
use bientot_platform::tray::{ScreenPoint, TrayHost};
use gpui::{
    App, AppContext as _, Bounds, DevicePixels, DisplayId, Global, Pixels, QuitMode, Size,
    TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, point, px, size,
};

use crate::flyout::Flyout;
use crate::host::{self, Today};
use crate::live::Tracking;
use crate::strings::{Key, Language};
use crate::widgets::Notice;
use crate::workday::DayView;
use crate::{platform, theme};

/// Gap between the flyout's panel and the taskbar or the screen edges, in physical pixels, as
/// in the Windows 11 flyouts.
const MARGIN: i32 = 12;

/// How far the flyout's shadow spreads past its panel, in logical pixels, where the app draws it:
/// as far as the margin, so that on X11 at 100 % and in a layer surface the window stops at the
/// edge of the work area. The faint tail of `elevation-modal` past it is cut.
const SHADOW_ROOM: Pixels = px(12.);

/// A click on the tray icon first takes the focus away from an open flyout, which closes it.
/// Without this delay the same click would open it again at once.
const REOPEN_DELAY: Duration = Duration::from_millis(300);

/// The size of the flyout's panel when its window was last fitted: the size it most likely has
/// at the next opening.
static LAST_PANEL: Mutex<Option<Size<Pixels>>> = Mutex::new(None);

#[derive(Default)]
struct FlyoutState {
    open: Option<Open>,
    closed_at: Option<Instant>,
}

/// The flyout's open window.
struct Open {
    window: WindowHandle<Flyout>,
    /// The flyout the window shows, which an action pressed in it updates while the window is
    /// being updated: no other update of the window can enter then.
    view: Option<WeakEntity<Flyout>>,
    /// Where it opened, which a later click or a change of the tray does not move.
    placement: FlyoutPlacement,
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

/// When the app quits: with its last window, unless a tray icon keeps it running after its
/// flyout, the only window, closes.
pub fn quit_mode(tray: TrayHost) -> QuitMode {
    match tray {
        TrayHost::Present => QuitMode::Explicit,
        TrayHost::Absent => QuitMode::Default,
    }
}

/// A tray host appeared or went away: the app switches to the tray or to window mode at once.
pub fn tray_changed(tray: TrayHost, cx: &mut App) {
    cx.global_mut::<Placement>().tray = tray;
    // Before a window closes below: an app the host now keeps must outlive it.
    cx.set_quit_mode(quit_mode(tray));
    match switch(tray, shown(cx)) {
        Switch::Keep => {}
        Switch::Open => open_flyout(cx),
        Switch::Reopen => reopen(cx),
        Switch::Close => {
            if let Some(open) = cx.default_global::<FlyoutState>().open.take() {
                open.window
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            }
        }
    }
}

/// Whether window mode shows the app now: no tray host shows its icon.
pub fn window_mode(cx: &App) -> bool {
    cx.try_global::<Placement>()
        .is_some_and(|placement| placement.tray == TrayHost::Absent)
}

/// The user closed the notice that says why the app shows in a window, at this launch or an
/// earlier one.
struct WindowModeNoticeClosed;

impl Global for WindowModeNoticeClosed {}

/// Keeps the flyout from telling again why the app shows in a window.
pub fn close_window_mode_notice(cx: &mut App) {
    cx.set_global(WindowModeNoticeClosed);
}

/// Whether the flyout tells why the app shows in a window: in window mode only, until the user
/// closed the notice. Never on Windows, whose tray is always there.
pub fn shows_window_mode_notice(cx: &App) -> bool {
    window_mode(cx) && !cx.has_global::<WindowModeNoticeClosed>()
}

/// The page of the notice's « Comment afficher l’icône »: how to have the desktop show the icon,
/// and what window mode does (T173 of specs/005-linux-support).
pub fn icon_help_url(language: Language) -> &'static str {
    match language {
        Language::French => "https://bientot.dev/aide/afficher-l-icone",
        Language::English => "https://bientot.dev/en/help/show-the-icon",
    }
}

/// What the shell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Nothing,
    Flyout,
    /// Window mode's window, `active` when the user is in it rather than elsewhere or minimized.
    Window {
        active: bool,
    },
}

/// What a change of the tray host does to what the shell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Switch {
    Keep,
    /// Shows the app, which nothing else would show.
    Open,
    /// Shows it again in the other form.
    Reopen,
    Close,
}

/// Without a host the app must show itself in a window; with one, the window the user is in
/// becomes the flyout, and a window they left goes, the icon taking its place.
fn switch(tray: TrayHost, shown: Shown) -> Switch {
    match (tray, shown) {
        (TrayHost::Absent, Shown::Nothing) => Switch::Open,
        (TrayHost::Absent, Shown::Flyout) => Switch::Reopen,
        (TrayHost::Present, Shown::Window { active: true }) => Switch::Reopen,
        (TrayHost::Present, Shown::Window { active: false }) => Switch::Close,
        (TrayHost::Absent, Shown::Window { .. }) | (TrayHost::Present, _) => Switch::Keep,
    }
}

fn shown(cx: &mut App) -> Shown {
    let Some(open) = cx.default_global::<FlyoutState>().open.as_ref() else {
        return Shown::Nothing;
    };
    let window = open.window;
    match open.placement {
        FlyoutPlacement::Window => {
            let active = window.update(cx, |_, window, _| window.is_window_active());
            Shown::Window {
                // GPUI holds the window while an action pressed in it runs, and then it cannot be
                // read: the user is in it.
                active: active.unwrap_or_else(|_| {
                    cx.windows()
                        .iter()
                        .any(|open| open.window_id() == window.window_id())
                }),
            }
        }
        _ => Shown::Flyout,
    }
}

/// Opens the flyout where it opens now, then closes the one open, so that the app never runs
/// without a window in between.
fn reopen(cx: &mut App) {
    let previous = cx.default_global::<FlyoutState>().open.take();
    open_new_flyout(None, cx);
    if let Some(previous) = previous {
        previous
            .window
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    }
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

/// How the flyout's window frames its panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelFrame {
    /// The system rounds and shadows the opaque window, which the panel fills.
    System,
    /// The window is transparent around the panel, which is rounded and casts its shadow in the
    /// `room` left on every side.
    Drawn { room: Pixels },
    /// The panel fills the opaque window, square: nothing could blend a rounded corner or a
    /// shadow, or the window has the frame of an ordinary window (window mode).
    Square,
}

impl PanelFrame {
    /// Until the shell frames it, as in the screenshots and the tests: the panel fills its
    /// window, rounded, with no room for a shadow.
    pub const UNFRAMED: Self = Self::Drawn { room: px(0.) };

    /// The transparent space between the panel and each edge of its window.
    pub fn room(self) -> Pixels {
        match self {
            Self::Drawn { room } => room,
            Self::System | Self::Square => px(0.),
        }
    }
}

/// How the flyout's window at `placement` frames its panel when `frame` draws its corners and its
/// shadow. Where the app positions it, the shadow takes no more room than the margin, so that the
/// window stays out of the panels around it: on X11, MARGIN physical pixels at the work area's
/// scale; in a layer surface, the margin, which then shrinks to keep the panel where it was.
pub fn panel_frame(frame: Frame, placement: FlyoutPlacement) -> PanelFrame {
    match (frame, placement) {
        (Frame::System, _) => PanelFrame::System,
        (Frame::Square, _) | (Frame::App, FlyoutPlacement::Window) => PanelFrame::Square,
        (Frame::App, FlyoutPlacement::At { work_area, .. }) => PanelFrame::Drawn {
            room: SHADOW_ROOM.min(px(MARGIN as f32 / work_area.scale)),
        },
        (Frame::App, FlyoutPlacement::Layer { margin, .. }) => PanelFrame::Drawn {
            room: SHADOW_ROOM.min(px(margin as f32)),
        },
        (Frame::App, FlyoutPlacement::CompositorPlaced) => PanelFrame::Drawn { room: SHADOW_ROOM },
    }
}

/// Who draws the flyout's frame on this desktop. Without a desktop, the app.
fn desktop_frame(cx: &App) -> Frame {
    cx.try_global::<Placement>()
        .map_or(Frame::App, |placement| placement.desktop.frame())
}

pub fn toggle_flyout(cx: &mut App) {
    let state = cx.default_global::<FlyoutState>();
    if let Some(open) = state.open.take() {
        state.closed_at = Some(Instant::now());
        open.window
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

/// Opens the flyout on the current day, or brings an open one to the front. Window mode's window,
/// once minimized or left, opens again: no compositor has to let the app bring it back.
pub fn open_flyout(cx: &mut App) {
    match shown(cx) {
        Shown::Nothing => open_new_flyout(None, cx),
        Shown::Window { active: false } => reopen(cx),
        Shown::Flyout | Shown::Window { active: true } => {
            if let Some(window) = open_window(cx) {
                window
                    .update(cx, |_, window, _| platform::activate_window(window))
                    .ok();
            }
        }
    }
}

/// The flyout's window, when it is open.
fn open_window(cx: &mut App) -> Option<WindowHandle<Flyout>> {
    cx.default_global::<FlyoutState>()
        .open
        .as_ref()
        .map(|open| open.window)
}

/// Opens the flyout, or brings it to the front, with `notice` above the day. A flyout it opens
/// shows `notice` from its first frame, at rest; in an open one `notice` comes in.
pub fn open_flyout_with(notice: Notice, cx: &mut App) {
    if cx.default_global::<FlyoutState>().open.is_none() {
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
    let open = |placement, cx: &mut App| {
        let (day, notice, alerts) = (day.clone(), notice.clone(), alerts.clone());
        let language = cx.try_global::<Language>().copied().unwrap_or_default();
        let frame = panel_frame(desktop_frame(cx), placement);
        cx.open_window(
            window_options(placement, frame, last_panel(), language),
            |window, cx| {
                cx.new(|cx| {
                    Flyout::new(day, tracking, notice, window, cx)
                        .with_alerts(alerts)
                        .framed(frame)
                })
            },
        )
    };
    let mut placement = placement(cx);
    let mut opened = open(placement, cx);
    // GPUI cannot tell before it tries whether the compositor has layer-shell: Mutter has none,
    // and then the compositor places the flyout (T044).
    if let (Err(_), FlyoutPlacement::Layer { .. }) = (&opened, placement) {
        placement = FlyoutPlacement::CompositorPlaced;
        opened = open(placement, cx);
    }
    match opened {
        Ok(window) => {
            app_shown();
            // `focus: true` only shows the window activated, and Windows keeps a window of a
            // background process out of the foreground: the flyout then never learns that a
            // click went elsewhere, and stays open. `activate_window` forces it to the front.
            window
                .update(cx, |_, window, _| platform::activate_window(window))
                .ok();
            cx.global_mut::<Today>().announced();
            let view = window.entity(cx).ok().map(|view| view.downgrade());
            cx.default_global::<FlyoutState>().open = Some(Open {
                window,
                view,
                placement,
            });
        }
        Err(_) => {
            host::log(DiagnosticLogLevel::Error, "flyout.open_failed", &[], cx);
            if platform::NO_WINDOW_MEANS_NO_GRAPHICS {
                let language = cx.try_global::<Language>().copied().unwrap_or_default();
                stop_without_graphics(language, cx);
            }
        }
    }
}

/// Tells the user why Bientôt stops when no window of its own can: on the standard error, which
/// release builds on Windows have no console for, and through the system.
pub fn alert(language: Language, key: Key) {
    let text = language.text(key);
    eprintln!("{text}");
    platform::show_fatal(language.text(Key::ShellWindowTitle), text);
}

/// Whether a window of Bientôt opened or its tray shows it yet in this process, or no window
/// could open. It lives on the thread
/// GPUI runs on rather than in the app, which is gone when `main` reads it for the exit status.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Windows {
    NoneYet,
    Opened,
    NoneOpens,
}

thread_local! {
    static WINDOWS: Cell<Windows> = const { Cell::new(Windows::NoneYet) };
}

/// A window of Bientôt opened, or the launch ended with its tray icon showing it: a window that
/// fails to open later stops nothing.
pub fn app_shown() {
    if WINDOWS.get() == Windows::NoneYet {
        WINDOWS.set(Windows::Opened);
    }
}

/// A window cannot open. Until one has, or the tray shows the app, nothing does, so Bientôt quits
/// rather than run on unseen, and says why. Afterwards the caller's log is all: the tray stays,
/// and the next click tries again.
pub fn stop_without_graphics(language: Language, cx: &mut App) {
    if WINDOWS.get() != Windows::NoneYet {
        return;
    }
    WINDOWS.set(Windows::NoneOpens);
    alert(language, Key::NoGraphicsText);
    quit_once_running(cx);
}

/// Whether Bientôt stops because no window could open: it then writes no ready file, and exits
/// with a failure, as a failed start does.
pub fn stopped_without_graphics() -> bool {
    WINDOWS.get() == Windows::NoneOpens
}

/// Quits once GPUI's event loop runs: on Linux the loop forgets a quit asked before it starts, as
/// from the launch's own callback.
pub fn quit_once_running(cx: &mut App) {
    cx.spawn(async |cx| cx.update(|cx| cx.quit())).detach();
}

/// The window the flyout opens in at `placement`, framing its panel `panel` large with `frame`.
/// Windows shows the window as soon as GPUI creates it, empty until its first frame fits it:
/// opened elsewhere than where that frame puts it, it would flash there, at the top left of the
/// primary monitor. So in a work area it opens in the corner the frame will fit it in, at the size
/// its panel last had.
fn window_options(
    placement: FlyoutPlacement,
    frame: PanelFrame,
    panel: Size<Pixels>,
    language: Language,
) -> WindowOptions {
    let room = frame.room();
    let popup = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            around(panel, room),
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        // Opaque unless the app draws the frame: on Windows, `Transparent` has Windows compose a
        // frosted layer behind the whole window. The window is the panel, and Windows draws its
        // shadow.
        window_background: match frame {
            PanelFrame::Drawn { .. } => WindowBackgroundAppearance::Transparent,
            PanelFrame::System | PanelFrame::Square => WindowBackgroundAppearance::Opaque,
        },
        ..Default::default()
    };
    match placement {
        FlyoutPlacement::At { work_area, corner } => {
            let client = flyout_bounds(bounds(work_area.bounds), corner, panel, work_area.scale);
            let client = client.dilate(physical(room, work_area.scale));
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(logical_bounds(
                    client,
                    work_area.scale,
                ))),
                display_id: Some(DisplayId::new(work_area.display)),
                ..popup
            }
        }
        FlyoutPlacement::Layer { corner, margin } => {
            layer_shell(popup, corner, margin - f32::from(room) as i32)
        }
        FlyoutPlacement::CompositorPlaced => popup,
        FlyoutPlacement::Window => WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(language.text(Key::ShellWindowTitle).into()),
                ..TitlebarOptions::default()
            }),
            kind: WindowKind::Normal,
            is_movable: true,
            is_minimizable: true,
            ..popup
        },
    }
}

/// `popup` as a layer-shell surface in `corner` of the output, `margin` logical pixels from both
/// edges, on the layer above windows, with the keyboard once the user points at it or clicks it
/// (A-3).
#[cfg(target_os = "linux")]
fn layer_shell(popup: WindowOptions, corner: Corner, margin: i32) -> WindowOptions {
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};

    let anchor = match corner {
        Corner::TopLeft => Anchor::TOP | Anchor::LEFT,
        Corner::TopRight => Anchor::TOP | Anchor::RIGHT,
        Corner::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
        Corner::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
    };
    let margin = px(margin as f32);
    WindowOptions {
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "bientot-flyout".into(),
            layer: Layer::Top,
            anchor,
            margin: Some((margin, margin, margin, margin)),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        }),
        ..popup
    }
}

/// GPUI has `WindowKind::LayerShell` only on Linux, and no other backend answers `Layer`.
#[cfg(not(target_os = "linux"))]
fn layer_shell(popup: WindowOptions, _corner: Corner, _margin: i32) -> WindowOptions {
    popup
}

/// The size of a window that leaves `room` around `panel` on every side.
fn around(panel: Size<Pixels>, room: Pixels) -> Size<Pixels> {
    size(panel.width + room * 2., panel.height + room * 2.)
}

/// `logical` in whole physical pixels at `scale`.
fn physical(logical: Pixels, scale: f32) -> i32 {
    (f32::from(logical) * scale).round() as i32
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
    let view = cx
        .default_global::<FlyoutState>()
        .open
        .as_ref()
        .and_then(|open| open.view.clone());
    if let Some(view) = view {
        view.update(cx, update).ok();
    }
}

/// Called when `window`, the flyout's, closed, so the next tray click reopens it. A window the
/// shell already replaced changes nothing.
pub fn flyout_closed(window: &Window, cx: &mut App) {
    let state = cx.default_global::<FlyoutState>();
    if state
        .open
        .as_ref()
        .is_some_and(|open| open.window.window_id() == window.window_handle().window_id())
    {
        state.open = None;
        state.closed_at = Some(Instant::now());
    }
}

/// The flyout is done with, on Esc or when the focus went elsewhere: it closes, and window mode's
/// window minimizes, since closing the app's only window would quit it.
pub fn dismiss(window: &mut Window, cx: &mut App) {
    let window_mode = cx
        .default_global::<FlyoutState>()
        .open
        .as_ref()
        .is_some_and(|open| {
            open.window.window_id() == window.window_handle().window_id()
                && open.placement == FlyoutPlacement::Window
        });
    if window_mode {
        window.minimize_window();
    } else {
        flyout_closed(window, cx);
        window.remove_window();
    }
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

/// Gives the window the measured panel size with `room` around it and, in a work area, puts the
/// panel in the corner next to the taskbar, like the Windows 11 flyouts: see [`flyout_bounds`].
pub fn fit_window(window: &mut Window, content: Size<Pixels>, room: Pixels, cx: &App) {
    *LAST_PANEL.lock().unwrap() = Some(content);
    if room > px(0.) {
        // A click on the shadow reaches what lies behind it, and closes the flyout as any click
        // outside it does. Only GPUI's Wayland backend takes an input region.
        window.set_input_region(Some(&[Bounds::new(point(room, room), content)]));
    }
    match placement(cx) {
        FlyoutPlacement::At { work_area, corner } => {
            let scale = window.scale_factor();
            let client = flyout_bounds(bounds(work_area.bounds), corner, content, scale);
            platform::place_window(window, client.dilate(physical(room, scale)));
        }
        FlyoutPlacement::Layer { .. }
        | FlyoutPlacement::CompositorPlaced
        | FlyoutPlacement::Window => window.resize(around(content, room)),
    }
}

/// Whether the window lost the size [`fit_window`] gave it. A layer surface takes the size of the
/// compositor's first configure, which GPUI applies over a resize asked before it came: the
/// flyout's first fitting on Wayland, when its panel is not the size it opened at.
pub fn lost_its_size(window: &Window, content: Size<Pixels>, room: Pixels, cx: &App) -> bool {
    let layer = cx
        .try_global::<FlyoutState>()
        .and_then(|state| state.open.as_ref())
        .is_some_and(|open| matches!(open.placement, FlyoutPlacement::Layer { .. }));
    let (size, fitted) = (window.bounds().size, around(content, room));
    // GPUI sizes a layer surface in whole logical pixels.
    layer
        && ((size.width - fitted.width).abs() >= px(1.)
            || (size.height - fitted.height).abs() >= px(1.))
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
/// pixels at `scale`: in its `corner`, MARGIN from the edges, and no taller than the work area
/// leaves room for with MARGIN at both ends.
pub fn flyout_bounds(
    work_area: Bounds<i32>,
    corner: Corner,
    panel: Size<Pixels>,
    scale: f32,
) -> Bounds<i32> {
    let width = (f32::from(panel.width) * scale).round() as i32;
    let tallest = work_area.size.height - 2 * MARGIN;
    let height = ((f32::from(panel.height) * scale).round() as i32).min(tallest);
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => work_area.left() + MARGIN,
        Corner::TopRight | Corner::BottomRight => work_area.right() - width - MARGIN,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => work_area.top() + MARGIN,
        Corner::BottomLeft | Corner::BottomRight => work_area.bottom() - height - MARGIN,
    };
    Bounds::new(point(x, y), size(width, height))
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
            flyout_bounds(area(0, 0, 1920, 1032), Corner::BottomRight, panel(600.), 1.),
            Bounds::new(point(1548, 420), size(360, 600)),
        );
    }

    #[test]
    fn a_flyout_taller_than_a_short_work_area_keeps_its_panel_a_margin_below_the_top() {
        // 1366 × 768 at 125 %, above a taskbar of 60 physical pixels.
        let bounds = flyout_bounds(
            area(0, 0, 1366, 708),
            Corner::BottomRight,
            panel(700.),
            1.25,
        );
        assert_eq!(bounds, Bounds::new(point(904, 12), size(450, 684)));
        assert_eq!(bounds.bottom(), 708 - 12);
    }

    #[test]
    fn a_panel_as_tall_as_a_short_work_area_allows_fills_it() {
        let tallest = panel_max_height(708, 1.25);
        assert_eq!(tallest, px(547.2));
        assert_eq!(
            flyout_bounds(
                area(0, 0, 1366, 708),
                Corner::BottomRight,
                panel(f32::from(tallest)),
                1.25
            ),
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
            flyout_bounds(
                area(0, 48, 1920, 1080),
                Corner::BottomRight,
                panel(2000.),
                1.
            ),
            Bounds::new(point(1548, 60), size(360, 1008)),
        );
        assert_eq!(
            flyout_bounds(
                area(48, 0, 1920, 1080),
                Corner::BottomRight,
                panel(600.),
                1.
            ),
            Bounds::new(point(1548, 468), size(360, 600)),
        );
    }

    #[test]
    fn a_panel_at_the_top_holds_the_flyout_a_margin_below_it_in_the_corner_of_the_click() {
        // xfce4-panel, 33 px tall at the top of a 1920 × 1080 screen (research.md, A-2).
        let work_area = area(0, 33, 1920, 1080);
        assert_eq!(
            flyout_bounds(work_area, Corner::TopRight, panel(600.), 1.),
            Bounds::new(point(1548, 45), size(360, 600)),
        );
        assert_eq!(
            flyout_bounds(work_area, Corner::TopLeft, panel(600.), 1.),
            Bounds::new(point(12, 45), size(360, 600)),
        );
        assert_eq!(
            flyout_bounds(work_area, Corner::BottomLeft, panel(600.), 1.),
            Bounds::new(point(12, 468), size(360, 600)),
        );
        let tall = flyout_bounds(work_area, Corner::TopRight, panel(2000.), 1.);
        assert_eq!(tall, Bounds::new(point(1548, 45), size(360, 1023)));
        assert_eq!(tall.bottom(), 1080 - 12);
    }

    #[test]
    fn the_flyout_opens_on_the_physical_pixels_its_first_frame_fits_it_to() {
        // 1366 × 768 at 125 %, and a monitor at 150 % left of the primary one.
        for (work_area, scale) in [
            (area(0, 0, 1366, 708), 1.25),
            (area(-2560, 0, 0, 1392), 1.5),
            (area(0, 0, 1920, 1032), 1.),
        ] {
            let fitted = flyout_bounds(work_area, Corner::BottomRight, panel(613.), scale);
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
        let options = window_options(
            FlyoutPlacement::At {
                work_area,
                corner: Corner::BottomRight,
            },
            PanelFrame::System,
            panel(613.),
            Language::French,
        );
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
        let options = window_options(
            FlyoutPlacement::CompositorPlaced,
            PanelFrame::System,
            panel(613.),
            Language::French,
        );
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
        let options = window_options(
            FlyoutPlacement::Window,
            PanelFrame::Square,
            panel(613.),
            Language::French,
        );
        assert_eq!(options.kind, WindowKind::Normal);
        assert!(options.is_minimizable);
        assert_eq!(
            options.titlebar.and_then(|titlebar| titlebar.title),
            Some("Bientôt".into())
        );
    }

    #[test]
    fn without_a_tray_host_the_app_shows_itself_in_a_window() {
        assert_eq!(switch(TrayHost::Absent, Shown::Nothing), Switch::Open);
        assert_eq!(switch(TrayHost::Absent, Shown::Flyout), Switch::Reopen);
        for active in [true, false] {
            assert_eq!(
                switch(TrayHost::Absent, Shown::Window { active }),
                Switch::Keep
            );
        }
    }

    #[test]
    fn a_tray_host_that_appears_turns_the_window_into_the_flyout_or_the_icon() {
        assert_eq!(
            switch(TrayHost::Present, Shown::Window { active: true }),
            Switch::Reopen
        );
        assert_eq!(
            switch(TrayHost::Present, Shown::Window { active: false }),
            Switch::Close
        );
        assert_eq!(switch(TrayHost::Present, Shown::Flyout), Switch::Keep);
        assert_eq!(switch(TrayHost::Present, Shown::Nothing), Switch::Keep);
    }

    #[test]
    fn only_a_tray_icon_keeps_the_app_running_without_a_window() {
        assert_eq!(quit_mode(TrayHost::Present), QuitMode::Explicit);
        assert_eq!(quit_mode(TrayHost::Absent), QuitMode::Default);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_layer_surface_is_anchored_to_its_corner_on_the_layer_above_windows() {
        use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer};

        let placement = FlyoutPlacement::Layer {
            corner: Corner::BottomLeft,
            margin: 12,
        };
        let frame = panel_frame(Frame::App, placement);
        let options = window_options(placement, frame, panel(613.), Language::French);
        let WindowKind::LayerShell(layer) = options.kind else {
            panic!("a layer surface");
        };
        assert_eq!(layer.anchor, Anchor::BOTTOM | Anchor::LEFT);
        assert_eq!(layer.layer, Layer::Top);
        // The shadow takes the margin: the panel stays 12 px from the edges.
        assert_eq!(frame.room(), px(12.));
        assert_eq!(layer.margin, Some((px(0.), px(0.), px(0.), px(0.))));
        assert_eq!(
            layer.keyboard_interactivity,
            KeyboardInteractivity::OnDemand
        );
        assert_eq!(layer.namespace, "bientot-flyout");
    }

    fn at(scale: f32) -> FlyoutPlacement {
        FlyoutPlacement::At {
            work_area: WorkArea {
                display: 0,
                bounds: Rect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1047,
                },
                scale,
            },
            corner: Corner::BottomRight,
        }
    }

    fn opening_bounds(options: &WindowOptions) -> Bounds<Pixels> {
        let Some(WindowBounds::Windowed(bounds)) = options.window_bounds else {
            panic!("a windowed flyout");
        };
        bounds
    }

    #[test]
    fn where_the_system_draws_the_frame_the_window_is_the_opaque_panel_wherever_it_opens() {
        for placement in [
            at(1.),
            at(1.5),
            FlyoutPlacement::CompositorPlaced,
            FlyoutPlacement::Window,
        ] {
            let frame = panel_frame(Frame::System, placement);
            assert_eq!(frame, PanelFrame::System);
            assert_eq!(frame.room(), px(0.));
            let options = window_options(placement, frame, panel(613.), Language::French);
            assert_eq!(
                options.window_background,
                WindowBackgroundAppearance::Opaque
            );
        }
        let options = window_options(at(1.), PanelFrame::System, panel(613.), Language::French);
        assert_eq!(
            opening_bounds(&options),
            logical_bounds(
                flyout_bounds(area(0, 0, 1920, 1047), Corner::BottomRight, panel(613.), 1.),
                1.
            ),
        );
    }

    #[test]
    fn where_the_app_draws_the_frame_the_window_is_transparent_with_room_for_the_shadow() {
        let frame = panel_frame(Frame::App, FlyoutPlacement::CompositorPlaced);
        assert_eq!(frame, PanelFrame::Drawn { room: px(12.) });
        let options = window_options(
            FlyoutPlacement::CompositorPlaced,
            frame,
            panel(613.),
            Language::French,
        );
        assert_eq!(
            options.window_background,
            WindowBackgroundAppearance::Transparent
        );
        assert_eq!(opening_bounds(&options).size, size(px(384.), px(637.)));
    }

    #[test]
    fn on_x11_the_shadow_stays_in_the_margin_so_the_window_stays_in_the_work_area() {
        for scale in [1., 1.25, 2.] {
            let placement = at(scale);
            let frame = panel_frame(Frame::App, placement);
            let room = frame.room();
            assert_eq!(physical(room, scale), MARGIN, "at {scale}");
            let options = window_options(placement, frame, panel(613.), Language::French);
            let opening = opening_bounds(&options).to_device_pixels(scale);
            let panel_at = flyout_bounds(
                area(0, 0, 1920, 1047),
                Corner::BottomRight,
                panel(613.),
                scale,
            );
            // The window ends on the work area's edges, the panel a margin inside them.
            assert_eq!(opening.right().0, 1920, "at {scale}");
            assert_eq!(opening.bottom().0, 1047, "at {scale}");
            assert_eq!(opening.origin.x.0, panel_at.origin.x - MARGIN, "at {scale}");
        }
        assert_eq!(panel_frame(Frame::App, at(1.)).room(), px(12.));
        assert_eq!(panel_frame(Frame::App, at(2.)).room(), px(6.));
    }

    #[test]
    fn without_a_compositing_manager_the_panel_fills_the_opaque_window_square() {
        for placement in [at(1.), FlyoutPlacement::CompositorPlaced] {
            let frame = panel_frame(Frame::Square, placement);
            assert_eq!(frame, PanelFrame::Square);
            let options = window_options(placement, frame, panel(613.), Language::French);
            assert_eq!(
                options.window_background,
                WindowBackgroundAppearance::Opaque
            );
            assert_eq!(opening_bounds(&options).size.height, px(613.));
        }
    }

    #[test]
    fn window_mode_has_the_frame_of_an_ordinary_window() {
        assert_eq!(
            panel_frame(Frame::App, FlyoutPlacement::Window),
            PanelFrame::Square
        );
    }

    #[test]
    fn without_a_frame_the_window_is_the_client_rectangle() {
        let client = Bounds::new(point(1548, 420), size(360, 600));
        assert_eq!(window_bounds(client, FrameInsets::default()), client);
    }

    /// The flyout opened and closed, then a click finds no graphics: the flyout does not open,
    /// Bientôt does not stop, and the next click opens it once graphics are back.
    #[cfg(feature = "screenshots")]
    #[test]
    fn a_flyout_that_cannot_open_after_a_window_did_stops_nothing() {
        use std::sync::Arc;

        use bientot_core::demo::DemoScenario;
        use gpui::HeadlessAppContext;

        use crate::assets::{self, Assets};
        use crate::demo::{self, DemoHost};
        use crate::host::Source;
        use crate::theme::ThemeMode;

        let graphics = Rc::new(Cell::new(true));
        let drivers = Rc::clone(&graphics);
        let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
            assets::FONT_FAMILY,
        ));
        let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(Assets), move || {
            anyhow::ensure!(drivers.get(), "no graphics driver");
            Ok(None)
        });
        let directory = demo::throwaway_directory();
        let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
        cx.update(|cx| {
            cx.set_global(ThemeMode::Light);
            cx.set_global(Language::English);
            assets::load_fonts(cx).unwrap();
            cx.set_global(Today::open(Source::Demo(Box::new(host)), Language::English).unwrap());
            open_flyout(cx);
        });
        assert_eq!(cx.update(|cx| cx.windows().len()), 1, "the flyout opens");
        cx.update(toggle_flyout);
        assert!(cx.update(|cx| cx.windows().is_empty()), "the flyout closes");

        graphics.set(false);
        cx.update(open_flyout);
        cx.run_until_parked();
        assert!(cx.update(|cx| cx.windows().is_empty()), "no window opens");
        assert!(!stopped_without_graphics());

        graphics.set(true);
        cx.update(open_flyout);
        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            1,
            "the next click opens it"
        );
        std::fs::remove_dir_all(directory).ok();
    }
}
