//! The notification-area icon and its menu, behind the `Tray` port, and the corner above it
//! where the flyout opens, behind the `Desktop` port. GPUI has no tray, so they come from the
//! `tray-icon` crate and its `muda` menus; their events reach the app through the channel
//! [`NotificationArea::new`] returns. What they show comes from the app, and the ids of the menu
//! items from [`bientot_platform::tray`].

use std::cell::{Cell, RefCell};
use std::num::NonZeroIsize;

use bientot_platform::placement::{Desktop, FlyoutPlacement, Rect, WorkArea};
use bientot_platform::tray::{
    MenuEntry, ScreenPoint, Tray, TrayEvent, TrayHost, TrayState, item_id,
};
use futures::channel::mpsc;
use gpui::{Bounds, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{
    ContextMenu as _, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, HMENU, SM_CXSMICON, SWP_NOACTIVATE, SWP_NOZORDER,
    SetMenuDefaultItem, SetWindowPos, USER_DEFAULT_SCREEN_DPI,
};
use windows::core::w;

use crate::platform::TrayEvents;
use crate::shell::{self, FrameInsets};

thread_local! {
    /// The icon's hidden window, once the icon shows: it lives as long as the app.
    static WINDOW: Cell<Option<NonZeroIsize>> = const { Cell::new(None) };
}

/// The notification-area icon, made at the first [`Tray::publish`]: what it shows is known only
/// once the day opened.
pub struct NotificationArea {
    shown: RefCell<Shown>,
}

enum Shown {
    Pending,
    Visible(Visible),
    /// Windows refused the icon: the app runs without one.
    Failed,
}

struct Visible {
    icon: TrayIcon,
    /// The pixmap the icon was made from, and the generation of the menu it shows.
    pixmap: (u32, Vec<u8>),
    generation: u64,
}

impl NotificationArea {
    /// The icon, and the channel its left clicks and chosen menu items arrive on.
    pub fn new() -> (Self, TrayEvents) {
        let (events, received) = mpsc::unbounded();
        let clicks = events.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                position,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let at = ScreenPoint {
                    x: position.x as i32,
                    y: position.y as i32,
                };
                clicks
                    .unbounded_send(TrayEvent::Activated { at: Some(at) })
                    .ok();
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            events
                .unbounded_send(TrayEvent::MenuChosen { id: event.id.0 })
                .ok();
        }));
        let tray = Self {
            shown: RefCell::new(Shown::Pending),
        };
        (tray, received)
    }
}

/// The icon's hidden window, on the UI thread, once the icon shows.
pub fn tray_window() -> Option<NonZeroIsize> {
    WINDOW.get()
}

impl<C> Tray<C> for NotificationArea {
    fn icon_sizes(&self) -> Vec<u32> {
        vec![small_icon_size()]
    }

    fn panel_is_dark(&self) -> Option<bool> {
        taskbar_is_dark()
    }

    /// The notification area is always there.
    fn host(&self) -> TrayHost {
        TrayHost::Present
    }

    /// Makes the icon at the first call; then sets the tooltip, and the icon and the menu only
    /// when they changed.
    fn publish(&self, state: TrayState<'_, C>) {
        let mut shown = self.shown.borrow_mut();
        match &mut *shown {
            Shown::Pending => match Visible::show(&state) {
                Ok(visible) => *shown = Shown::Visible(visible),
                Err(error) => {
                    log::error!("cannot create the notification-area icon: {error:#}");
                    *shown = Shown::Failed;
                }
            },
            Shown::Visible(visible) => {
                if let Err(error) = visible.update(&state) {
                    log::error!("cannot update the notification-area icon: {error:#}");
                }
            }
            Shown::Failed => {}
        }
    }
}

impl Visible {
    fn show<C>(state: &TrayState<'_, C>) -> anyhow::Result<Self> {
        let pixmap = pixmap(state)?;
        let icon = TrayIconBuilder::new()
            .with_icon(icon(pixmap)?)
            .with_tooltip(state.tooltip)
            .with_menu(Box::new(menu(state.menu, state.generation)?))
            .with_menu_on_left_click(false)
            .build()?;
        WINDOW.set(NonZeroIsize::new(icon.window_handle() as isize));
        Ok(Self {
            icon,
            pixmap: (pixmap.0, pixmap.1.to_vec()),
            generation: state.generation,
        })
    }

    fn update<C>(&mut self, state: &TrayState<'_, C>) -> anyhow::Result<()> {
        let pixmap = pixmap(state)?;
        if (self.pixmap.0, self.pixmap.1.as_slice()) != pixmap {
            self.icon.set_icon(Some(icon(pixmap)?))?;
            self.pixmap = (pixmap.0, pixmap.1.to_vec());
        }
        self.icon.set_tooltip(Some(state.tooltip))?;
        if self.generation != state.generation {
            self.icon
                .set_menu(Some(Box::new(menu(state.menu, state.generation)?)));
            self.generation = state.generation;
        }
        Ok(())
    }
}

/// The one size [`Tray::icon_sizes`] asks for.
fn pixmap<'a, C>(state: &TrayState<'a, C>) -> anyhow::Result<(u32, &'a [u8])> {
    state
        .icon
        .pixmaps
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("no icon to show"))
}

fn icon((size, rgba): (u32, &[u8])) -> anyhow::Result<Icon> {
    Ok(Icon::from_rgba(rgba.to_vec(), size, size)?)
}

/// The size of a notification-area icon at the primary monitor's scale, where the taskbar shows
/// it: 16 pixels at 100 %, 24 at 150 %. Given a larger icon, Windows shrinks it without smoothing.
fn small_icon_size() -> u32 {
    let (mut dpi, mut dpi_y) = (USER_DEFAULT_SCREEN_DPI, USER_DEFAULT_SCREEN_DPI);
    // SAFETY: the out-pointers are live locals; the primary monitor always exists.
    let size = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y).is_err() {
            dpi = USER_DEFAULT_SCREEN_DPI;
        }
        GetSystemMetricsForDpi(SM_CXSMICON, dpi)
    };
    u32::try_from(size)
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(16)
}

/// Whether the taskbar is dark: its colour mode, « Choisir votre mode Windows » in the settings,
/// can differ from the apps' mode. `None` when Windows does not say.
fn taskbar_is_dark() -> Option<bool> {
    let mut light = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the key and value names are static NUL-terminated strings; `light` holds the
    // `size` bytes the call may write.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&raw mut size),
        )
    };
    read.is_ok().then_some(light == 0)
}

/// Hands `append` the items of `entries`, the entries at `path` in the menu of `generation`.
fn append_entries<C>(
    entries: &[MenuEntry<C>],
    generation: u64,
    path: &[usize],
    append: &dyn Fn(&dyn IsMenuItem) -> tray_icon::menu::Result<()>,
) -> anyhow::Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        let path = [path, &[index]].concat();
        match entry {
            MenuEntry::Item { label, enabled, .. } => append(&MenuItem::with_id(
                item_id(generation, &path),
                label,
                *enabled,
                None,
            ))?,
            MenuEntry::Submenu { label, items } => {
                let submenu = Submenu::new(label, true);
                append_entries(items, generation, &path, &|item| submenu.append(item))?;
                append(&submenu)?;
            }
            MenuEntry::Separator => append(&PredefinedMenuItem::separator())?,
        }
    }
    Ok(())
}

fn menu<C>(entries: &[MenuEntry<C>], generation: u64) -> anyhow::Result<Menu> {
    let menu = Menu::new();
    append_entries(entries, generation, &[], &|item| menu.append(item))?;
    // « Ouvrir WorkTime » is the default item, in bold, as in the .NET app.
    // SAFETY: the menu owns this handle and outlives the call.
    unsafe { SetMenuDefaultItem(HMENU(menu.hpopupmenu() as *mut _), 0, 1)? };
    Ok(menu)
}

/// The corner of the work area next to the taskbar, where the Windows 11 flyouts open, on the
/// monitor the icon was last clicked on.
pub struct TaskbarCorner;

impl Desktop for TaskbarCorner {
    /// Where Windows does not tell the monitor's work area or its scale, the window opens where
    /// GPUI puts it. The notification area is always there.
    fn flyout_placement(&self, click: Option<ScreenPoint>, _tray: TrayHost) -> FlyoutPlacement {
        match clicked_monitor(click) {
            Some(work_area) => FlyoutPlacement::At { work_area },
            None => FlyoutPlacement::CompositorPlaced,
        }
    }

    fn usable_height(&self, click: Option<ScreenPoint>) -> Option<i32> {
        clicked_monitor(click).map(|monitor| monitor.bounds.height)
    }

    /// DWM rounds the window's corners and draws its shadow.
    fn system_draws_frame(&self) -> bool {
        true
    }
}

/// The work area of the monitor at `click`, or of the primary one before any click. `None` when
/// Windows does not tell its work area or its scale.
fn clicked_monitor(click: Option<ScreenPoint>) -> Option<WorkArea> {
    let ScreenPoint { x, y } = click.unwrap_or(ScreenPoint { x: 0, y: 0 });
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi, mut dpi_y) = (0, 0);
    // SAFETY: each out pointer is a local.
    let monitor = unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTOPRIMARY);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y).ok()?;
        monitor
    };
    let RECT {
        left,
        top,
        right,
        bottom,
    } = info.rcWork;
    Some(WorkArea {
        // GPUI's id for a monitor is its handle.
        display: monitor.0 as u64,
        bounds: Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        },
        scale: dpi as f32 / USER_DEFAULT_SCREEN_DPI as f32,
    })
}

/// Moves the flyout's `window` so that its client area covers `client`, in physical pixels,
/// with DWM's round corners and without its border.
pub fn place_window(window: &mut Window, client: Bounds<i32>) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    // When Windows does not tell the frame, places the window as if it had none: the frame then
    // covers the panel's edges, but the window still goes in the corner.
    let outer = shell::window_bounds(client, frame_insets(hwnd).unwrap_or_default());
    // DWM's round corner is 8 px, the panel's `radius::LG`: the two must stay equal.
    let corners = DWMWCP_ROUND;
    // The panel draws its own border; DWM's would double it.
    let border = DWMWA_COLOR_NONE;
    // Results ignored: before Windows 11 DWM refuses both attributes, and the window then
    // keeps square corners and its usual border, which is harmless.
    // SAFETY: `hwnd` is the flyout's live window, and each pointer and size describe a local
    // that outlives the call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corners).cast(),
            size_of_val(&corners) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const border).cast(),
            size_of_val(&border) as u32,
        );
    }
    // SAFETY: `hwnd` is the flyout's live window.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            outer.origin.x,
            outer.origin.y,
            outer.size.width,
            outer.size.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// How far the window's edges lie outside its client area. GPUI creates the flyout as an
/// overlapped window, to which Windows adds a frame; the panel fills the client area only.
fn frame_insets(hwnd: HWND) -> Option<FrameInsets> {
    let mut window = RECT::default();
    let mut client = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: `hwnd` is the flyout's live window, and each out pointer is a local.
    unsafe {
        GetWindowRect(hwnd, &mut window).ok()?;
        GetClientRect(hwnd, &mut client).ok()?;
        ClientToScreen(hwnd, &mut origin).ok().ok()?;
    }
    Some(FrameInsets {
        left: origin.x - window.left,
        top: origin.y - window.top,
        right: window.right - origin.x - client.right,
        bottom: window.bottom - origin.y - client.bottom,
    })
}
