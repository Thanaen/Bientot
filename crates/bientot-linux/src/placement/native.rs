use std::cell::RefCell;

use bientot_platform::placement::{Desktop, FlyoutPlacement};
use bientot_platform::tray::{ScreenPoint, TrayHost};

use super::{Layout, WaylandShell, X11, desktops, wayland};

/// The desktop of this session, as `WAYLAND_DISPLAY` and `DISPLAY` name it, the way GPUI chooses.
pub struct LinuxDesktop {
    session: Session,
    /// `XDG_CURRENT_DESKTOP`, which decides the default corner on Wayland.
    desktops: Vec<String>,
}

enum Session {
    X11 {
        x11: Box<X11>,
        scale: f32,
        /// Read when the flyout was last placed: the frames that follow ask for its height.
        layout: RefCell<Option<Layout>>,
    },
    Wayland(WaylandShell),
    None,
}

impl LinuxDesktop {
    pub fn of_this_session() -> Self {
        let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let session = if set("WAYLAND_DISPLAY") {
            Session::Wayland(wayland::read().unwrap_or(WaylandShell {
                layer_shell: false,
                outputs: Vec::new(),
            }))
        } else if set("DISPLAY") {
            X11::connect(None).map_or(Session::None, |x11| {
                let scale = x11.scale();
                Session::X11 {
                    x11: Box::new(x11),
                    scale,
                    layout: RefCell::default(),
                }
            })
        } else {
            Session::None
        };
        Self {
            session,
            desktops: desktops(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()),
        }
    }

    /// The X server, which moves and focuses the flyout's window, on X11 only.
    pub fn x11(&self) -> Option<&X11> {
        match &self.session {
            Session::X11 { x11, .. } => Some(x11),
            Session::Wayland(_) | Session::None => None,
        }
    }
}

impl Desktop for LinuxDesktop {
    /// Without a host, an ordinary window (FR-012), whose height the X11 layout read here still
    /// bounds.
    fn flyout_placement(&self, click: Option<ScreenPoint>, tray: TrayHost) -> FlyoutPlacement {
        let placement = match &self.session {
            Session::X11 { x11, scale, layout } => {
                let read = x11.layout().ok();
                let placement = read
                    .as_ref()
                    .and_then(|read| read.placement(click, x11.screen(), *scale));
                *layout.borrow_mut() = read;
                placement.unwrap_or(FlyoutPlacement::CompositorPlaced)
            }
            Session::Wayland(shell) => shell.placement(click, &self.desktops),
            Session::None => FlyoutPlacement::CompositorPlaced,
        };
        match tray {
            TrayHost::Present => placement,
            TrayHost::Absent => FlyoutPlacement::Window,
        }
    }

    fn usable_height(&self, click: Option<ScreenPoint>) -> Option<i32> {
        match &self.session {
            Session::X11 { layout, .. } => {
                let (_, area) = layout.borrow().as_ref()?.work_area_at(click)?;
                Some(area.height)
            }
            Session::Wayland(shell) => shell.usable_height(),
            Session::None => None,
        }
    }

    /// No Linux desktop rounds and shadows a window without decorations: the app draws both
    /// (T064).
    fn system_draws_frame(&self) -> bool {
        false
    }
}
