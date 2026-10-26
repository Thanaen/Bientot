//! The notification-area icon and its menu. GPUI has none, so they come from the `tray-icon`
//! crate and its `muda` menus; their events reach GPUI through channels read on the main thread.
//! What they show comes from `tray_model`.

use std::num::NonZeroIsize;
use std::sync::Mutex;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Bounds, DisplayId, Global, Pixels, Size, Window, point};
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

use crate::host::{self, Today};
use crate::shell::{self, FrameInsets};
use crate::tray_art::{self, Taskbar};
use crate::tray_model::{self, MenuEntry};
use crate::workday;

/// Where the icon was last clicked, in physical pixels, to open the flyout on that monitor.
static LAST_CLICK: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// The icon, and the menu it shows. Each menu built gets a new generation, which prefixes the
/// ids of its items: a click on an item of a menu since replaced is ignored, see
/// [`tray_model::item_id`].
struct Tray {
    icon: TrayIcon,
    shown: Drawn,
    menu: Vec<MenuEntry>,
    generation: u64,
}

impl Global for Tray {}

/// The icon as drawn: the day's icon at the taskbar's size and theme, which change with the
/// display scale and the Windows colour mode.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Drawn {
    icon: workday::TrayIcon,
    size: u32,
    taskbar: Taskbar,
}

impl Drawn {
    fn now(icon: workday::TrayIcon) -> Self {
        Self {
            icon,
            size: small_icon_size(),
            taskbar: taskbar_theme(),
        }
    }

    fn icon(self) -> anyhow::Result<Icon> {
        let rgba = tray_art::rgba(self.icon, self.size, self.taskbar)?;
        Ok(Icon::from_rgba(rgba, self.size, self.size)?)
    }
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

/// The taskbar's colour mode, « Choisir votre mode Windows » in the settings, which can differ
/// from the apps' mode. Dark when Windows does not say.
fn taskbar_theme() -> Taskbar {
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
    if read.is_ok() && light != 0 {
        Taskbar::Light
    } else {
        Taskbar::Dark
    }
}

/// Hands `append` the items of `entries`, the entries at `path` in the menu of `generation`.
fn append_entries(
    entries: &[MenuEntry],
    generation: u64,
    path: &[usize],
    append: &dyn Fn(&dyn IsMenuItem) -> tray_icon::menu::Result<()>,
) -> anyhow::Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        let path = [path, &[index]].concat();
        match entry {
            MenuEntry::Item { label, enabled, .. } => append(&MenuItem::with_id(
                tray_model::item_id(generation, &path),
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

fn menu(entries: &[MenuEntry], generation: u64) -> anyhow::Result<Menu> {
    let menu = Menu::new();
    append_entries(entries, generation, &[], &|item| menu.append(item))?;
    // « Ouvrir WorkTime » is the default item, in bold, as in the .NET app.
    // SAFETY: the menu owns this handle and outlives the call.
    unsafe { SetMenuDefaultItem(HMENU(menu.hpopupmenu() as *mut _), 0, 1)? };
    Ok(menu)
}

/// Shows the icon, the tooltip and the menu of the current day, rebuilding the menu only when
/// it changed.
pub fn show(cx: &mut App) {
    if !cx.has_global::<Tray>() {
        return;
    }
    let today = cx.global::<Today>();
    let (day, entries) = (today.view().clone(), today.menu());
    let tray = cx.global_mut::<Tray>();
    let shown = (|| -> anyhow::Result<()> {
        let drawn = Drawn::now(day.tray_icon);
        if tray.shown != drawn {
            tray.icon.set_icon(Some(drawn.icon()?))?;
            tray.shown = drawn;
        }
        tray.icon.set_tooltip(Some(&day.tray_tooltip))?;
        if tray.menu != entries {
            let generation = tray.generation + 1;
            tray.icon
                .set_menu(Some(Box::new(menu(&entries, generation)?)));
            tray.menu = entries;
            tray.generation = generation;
        }
        Ok(())
    })();
    if let Err(error) = shown {
        log::error!("cannot update the notification-area icon: {error:#}");
    }
}

/// Shows the icon of the current day, opens the flyout on a left click and the menu on a right
/// click.
pub fn install(cx: &mut App) -> anyhow::Result<()> {
    let today = cx.global::<Today>();
    let (day, entries) = (today.view().clone(), today.menu());
    let drawn = Drawn::now(day.tray_icon);
    let tray = TrayIconBuilder::new()
        .with_icon(drawn.icon()?)
        .with_tooltip(&day.tray_tooltip)
        .with_menu(Box::new(menu(&entries, 0)?))
        .with_menu_on_left_click(false)
        .build()?;
    cx.set_global(Tray {
        icon: tray,
        shown: drawn,
        menu: entries,
        generation: 0,
    });

    let (clicks, mut received) = mpsc::unbounded();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            position,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            clicks
                .unbounded_send((position.x as i32, position.y as i32))
                .ok();
        }
    }));
    cx.spawn(async move |cx| {
        while let Some(position) = received.next().await {
            *LAST_CLICK.lock().unwrap() = Some(position);
            cx.update(shell::toggle_flyout);
        }
    })
    .detach();

    let (chosen, mut received) = mpsc::unbounded();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        chosen.unbounded_send(event.id.0).ok();
    }));
    cx.spawn(async move |cx| {
        while let Some(id) = received.next().await {
            cx.update(|cx| {
                let tray = cx.global::<Tray>();
                if let Some(command) = tray_model::chosen(&tray.menu, tray.generation, &id) {
                    host::run(&command, cx);
                }
            });
        }
    })
    .detach();
    Ok(())
}

/// The icon's hidden window, on the UI thread for as long as the icon lives.
pub fn window_handle(cx: &App) -> Option<NonZeroIsize> {
    NonZeroIsize::new(cx.try_global::<Tray>()?.icon.window_handle() as isize)
}

/// The monitor the icon was last clicked on, where the flyout opens.
pub(crate) struct ClickedMonitor {
    pub display: DisplayId,
    /// In physical pixels.
    pub work_area: Bounds<i32>,
    pub scale: f32,
}

/// The monitor the icon was last clicked on, or the primary one before any click. `None` when
/// Windows does not tell its work area or its scale.
pub(crate) fn clicked_monitor() -> Option<ClickedMonitor> {
    let (x, y) = LAST_CLICK.lock().unwrap().unwrap_or((0, 0));
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
    Some(ClickedMonitor {
        // GPUI's id for a monitor is its handle.
        display: DisplayId::new(monitor.0 as u64),
        work_area: Bounds::from_corners(point(left, top), point(right, bottom)),
        scale: dpi as f32 / USER_DEFAULT_SCREEN_DPI as f32,
    })
}

/// The work area of the monitor the icon was last clicked on, in physical pixels. `None` when
/// Windows does not tell it.
pub(crate) fn work_area() -> Option<Bounds<i32>> {
    clicked_monitor().map(|monitor| monitor.work_area)
}

/// Sizes the flyout to its content and puts it in the corner of the work area next to the
/// taskbar, like the Windows 11 flyouts: see [`shell::flyout_bounds`]. Only sizes it when
/// Windows does not tell the work area. When Windows does not tell the frame either, places
/// the window as if it had none: the frame then covers the panel's edges, but the window
/// still goes in the corner rather than at the primary monitor's top left.
pub fn place_above_taskbar(window: &mut Window, content: Size<Pixels>) {
    let Some(work_area) = work_area() else {
        window.resize(content);
        return;
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    let client = shell::flyout_bounds(work_area, content, window.scale_factor());
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
