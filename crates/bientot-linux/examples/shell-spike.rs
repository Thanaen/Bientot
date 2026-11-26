//! The tray and flyout spike of Lot 4 (specs/005-linux-support, T040 to T044). Never shipped.
//!
//! A StatusNotifierItem whose primary click opens a GPUI window of the flyout's size by one
//! placement of `contracts/linux-backends.md`, logging to standard error what assumptions A-1 to
//! A-5 need: the click's coordinates, the window's bounds, its activation, the keys it receives
//! and why it closed.
//!
//! ```text
//! cargo run -p bientot-linux --example shell-spike -- <placement> [--open]
//! ```
//!
//! Placements:
//!
//! - `auto`: `layer` on Wayland, `popup` on X11;
//! - `popup`: `WindowKind::PopUp`; on X11 an override-redirect window at the bounds computed from
//!   `_NET_WORKAREA` and RandR, on Wayland an ordinary toplevel;
//! - `popup-focus`: `popup`, then the X11 input focus set on it by the spike (a remedy for A-1);
//! - `normal`: the A-1 fallback on X11, an undecorated `Normal` window with skip-taskbar and above,
//!   moved to the computed bounds;
//! - `layer`: a layer-shell surface, layer `Top`, keyboard `OnDemand`, in the corner nearest the
//!   click (A-3);
//! - `layer-exclusive`: `layer` with keyboard `Exclusive`, a remedy for A-3;
//! - `compositor`: an undecorated `Normal` window the compositor places (A-4).
//!
//! `--open` opens the window at start, as from a click at an unknown position, for a session with
//! no tray host (GNOME without the AppIndicator extension).

#[cfg(target_os = "linux")]
fn main() {
    spike::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The shell spike runs on Linux only.");
}

#[cfg(target_os = "linux")]
mod spike {
    use std::rc::Rc;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use bientot_linux::placement::{Layout, X11, nearest_corner};
    use bientot_platform::placement::{Corner, Rect};
    use bientot_platform::tray::ScreenPoint;
    use futures::StreamExt;
    use futures::channel::mpsc::{UnboundedSender, unbounded};
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    use gpui::{
        App, AppContext, Bounds, Context, FocusHandle, Global, InteractiveElement, IntoElement,
        KeyDownEvent, MouseButton, ParentElement, Pixels, QuitMode, Render, Styled, Subscription,
        Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind, WindowOptions, div,
        point, px, rgb, size,
    };
    use ksni::blocking::TrayMethods;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask,
        InputFocus, PropMode,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// The flyout's width (`theme::size::FLYOUT_WIDTH`) and the height the app opens it at
    /// before its first fitting.
    const FLYOUT: (i32, i32) = (360, 400);
    /// The space the app keeps between the panel and the work area's edges.
    const MARGIN: i32 = 12;

    static START: OnceLock<Instant> = OnceLock::new();

    macro_rules! log {
        ($($arg:tt)*) => {
            eprintln!(
                "{:>7}ms {}",
                START.get_or_init(Instant::now).elapsed().as_millis(),
                format!($($arg)*)
            )
        };
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Placement {
        PopUp,
        PopUpFocus,
        Normal,
        Layer(KeyboardInteractivity),
        Compositor,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Session {
        X11,
        Wayland,
    }

    enum TrayEvent {
        Activate { x: i32, y: i32 },
        SecondaryActivate { x: i32, y: i32 },
    }

    struct Spike {
        placement: Placement,
        session: Session,
        desktop: String,
        x11: Option<Rc<X11Probe>>,
        flyout: Option<WindowHandle<Flyout>>,
        _tray: Option<ksni::blocking::Handle<SpikeTray>>,
    }

    impl Global for Spike {}

    pub fn main() {
        log!("spike.start pid={}", std::process::id());
        let mut arguments = std::env::args().skip(1);
        let session = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Session::Wayland
        } else {
            Session::X11
        };
        let placement = match arguments.next().as_deref() {
            None | Some("auto") if session == Session::Wayland => {
                Placement::Layer(KeyboardInteractivity::OnDemand)
            }
            None | Some("auto") => Placement::PopUp,
            Some("popup") => Placement::PopUp,
            Some("popup-focus") => Placement::PopUpFocus,
            Some("normal") => Placement::Normal,
            Some("layer") => Placement::Layer(KeyboardInteractivity::OnDemand),
            Some("layer-exclusive") => Placement::Layer(KeyboardInteractivity::Exclusive),
            Some("compositor") => Placement::Compositor,
            Some(other) => {
                eprintln!("unknown placement {other}");
                std::process::exit(2);
            }
        };
        let open_at_start = match arguments.next().as_deref() {
            None => false,
            Some("--open") => true,
            Some(other) => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        };
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase();
        log!("spike.session session={session:?} desktop={desktop:?} placement={placement:?}");

        let x11 = (session == Session::X11)
            .then(|| X11Probe::connect().map(Rc::new))
            .and_then(|probe| {
                probe
                    .inspect_err(|error| log!("x11.probe_error error={error}"))
                    .ok()
            });

        let (sender, mut receiver) = unbounded();
        let tray = match (SpikeTray { sender }).assume_sni_available(true).spawn() {
            Ok(handle) => {
                log!("tray.spawned");
                Some(handle)
            }
            Err(error) => {
                log!("tray.error error={error:?}");
                None
            }
        };

        gpui_platform::application()
            .with_quit_mode(QuitMode::Explicit)
            .run(move |cx: &mut App| {
                cx.set_global(Spike {
                    placement,
                    session,
                    desktop,
                    x11,
                    flyout: None,
                    _tray: tray,
                });
                cx.spawn(async move |cx| {
                    while let Some(event) = receiver.next().await {
                        cx.update(|cx| on_tray(event, cx));
                    }
                })
                .detach();
                if open_at_start {
                    open(None, cx);
                }
            });
    }

    fn on_tray(event: TrayEvent, cx: &mut App) {
        match event {
            TrayEvent::Activate { x, y } => {
                log!("tray.activate x={x} y={y}");
                if let Some(flyout) = cx.global_mut::<Spike>().flyout.take() {
                    flyout
                        .update(cx, |_, window, _| {
                            log!("flyout.closed reason=tray");
                            window.remove_window();
                        })
                        .ok();
                    return;
                }
                // `0, 0` is how hosts say they do not know (contracts/linux-backends.md).
                let click = (x != 0 || y != 0).then_some(ScreenPoint { x, y });
                open(click, cx);
            }
            TrayEvent::SecondaryActivate { x, y } => {
                log!("tray.secondary_activate x={x} y={y}");
            }
        }
    }

    fn open(click: Option<ScreenPoint>, cx: &mut App) {
        let spike = cx.global::<Spike>();
        let (placement, session) = (spike.placement, spike.session);
        let x11 = spike.x11.clone();
        let (options, physical) = match (placement, session) {
            (Placement::Layer(keyboard), _) => (layer_options(click, keyboard, cx), None),
            (Placement::Compositor, _) => (compositor_options(), None),
            (_, Session::Wayland) => (popup_options(None), None),
            (Placement::PopUp | Placement::PopUpFocus, Session::X11) => {
                let physical = x11_bounds(click);
                (popup_options(physical), physical)
            }
            (Placement::Normal, Session::X11) => {
                let physical = x11_bounds(click);
                (
                    WindowOptions {
                        kind: WindowKind::Normal,
                        window_decorations: Some(WindowDecorations::Client),
                        ..popup_options(physical)
                    },
                    physical,
                )
            }
        };
        log!(
            "flyout.open placement={placement:?} kind={} requested={}",
            kind_name(&options.kind),
            options
                .window_bounds
                .map(|bounds| show_bounds(bounds.get_bounds()))
                .unwrap_or_default()
        );
        let opened = cx.open_window(options, |window, cx| {
            cx.new(|cx| Flyout::new(placement, window, cx))
        });
        let flyout = match opened {
            Ok(flyout) => flyout,
            Err(error) => {
                log!("flyout.open_failed error={error:#}");
                return;
            }
        };
        cx.global_mut::<Spike>().flyout = Some(flyout);
        flyout
            .update(cx, |_, window, _| {
                let xid = x11_window(window);
                log!(
                    "flyout.opened bounds={} scale={} active={} x11_window={xid:?}",
                    show_bounds(window.bounds()),
                    window.scale_factor(),
                    window.is_window_active()
                );
                if let (Some(probe), Some(xid)) = (&x11, xid) {
                    match placement {
                        Placement::PopUpFocus => probe.focus(xid),
                        Placement::Normal => probe.undecorated_above(xid),
                        _ => {}
                    }
                    if let Some(rect) = physical
                        && (placement == Placement::Normal || window.scale_factor() != 1.)
                    {
                        probe.place(xid, rect);
                    }
                    probe.report("opened", xid);
                }
                window.activate_window();
                log!("flyout.activate_window_called");
            })
            .ok();
        // The compositor answers later: report again once it has.
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            flyout
                .update(cx, |_, window, _| {
                    log!(
                        "flyout.after_500ms bounds={} active={}",
                        show_bounds(window.bounds()),
                        window.is_window_active()
                    );
                    if let (Some(probe), Some(xid)) = (&x11, x11_window(window)) {
                        probe.report("after_500ms", xid);
                    }
                })
                .ok();
        })
        .detach();
    }

    /// The flyout's rectangle on X11, in physical pixels: the work area of the clicked monitor,
    /// the corner nearest the click, `MARGIN` from both edges.
    fn x11_bounds(click: Option<ScreenPoint>) -> Option<Rect> {
        let desktop: Layout = X11::connect(None)
            .and_then(|x11| x11.layout())
            .inspect_err(|error| log!("x11.desktop_error error={error}"))
            .ok()?;
        let (monitor, area) = desktop.work_area_at(click)?;
        let corner = nearest_corner(area, click);
        let rect = rect_in_corner(area, corner);
        log!(
            "x11.desktop monitors={:?} net_workarea={:?} clicked_monitor={:?} work_area={area:?} corner={corner:?} flyout={rect:?}",
            desktop.monitors,
            desktop.net_workarea,
            monitor.bounds
        );
        Some(rect)
    }

    fn rect_in_corner(area: Rect, corner: Corner) -> Rect {
        let (width, height) = (FLYOUT.0, FLYOUT.1.min(area.height - 2 * MARGIN));
        let left = area.x + MARGIN;
        let right = area.x + area.width - MARGIN - width;
        let top = area.y + MARGIN;
        let bottom = area.y + area.height - MARGIN - height;
        let (x, y) = match corner {
            Corner::TopLeft => (left, top),
            Corner::TopRight => (right, top),
            Corner::BottomLeft => (left, bottom),
            Corner::BottomRight => (right, bottom),
        };
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// A pop-up at `physical`, read at a scale of 1: GPUI gives the window's scale only once it
    /// exists, and the spike moves it with X11 when the scale is another.
    fn popup_options(physical: Option<Rect>) -> WindowOptions {
        let bounds = physical.map_or(
            Bounds::new(
                point(px(0.), px(0.)),
                size(px(FLYOUT.0 as f32), px(FLYOUT.1 as f32)),
            ),
            |rect| {
                Bounds::new(
                    point(px(rect.x as f32), px(rect.y as f32)),
                    size(px(rect.width as f32), px(rect.height as f32)),
                )
            },
        );
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            app_id: Some("dev.bientot.Bientot".into()),
            ..Default::default()
        }
    }

    fn compositor_options() -> WindowOptions {
        WindowOptions {
            kind: WindowKind::Normal,
            window_decorations: Some(WindowDecorations::Client),
            ..popup_options(None)
        }
    }

    fn layer_options(
        click: Option<ScreenPoint>,
        keyboard: KeyboardInteractivity,
        cx: &App,
    ) -> WindowOptions {
        let spike = cx.global::<Spike>();
        // GPUI's Wayland backend names no primary display; the spike takes the first output.
        let output = cx.displays().first().map(|display| {
            let bounds = display.bounds();
            Rect {
                x: f32::from(bounds.origin.x) as i32,
                y: f32::from(bounds.origin.y) as i32,
                width: f32::from(bounds.size.width) as i32,
                height: f32::from(bounds.size.height) as i32,
            }
        });
        let corner = match (click, output) {
            (Some(_), Some(output)) => nearest_corner(output, click),
            _ if spike.desktop.split(':').any(|name| name == "kde") => Corner::BottomRight,
            _ => Corner::TopRight,
        };
        log!("layer.corner output={output:?} click={click:?} corner={corner:?}");
        let anchor = match corner {
            Corner::TopLeft => Anchor::TOP | Anchor::LEFT,
            Corner::TopRight => Anchor::TOP | Anchor::RIGHT,
            Corner::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
            Corner::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
        };
        let margin = px(MARGIN as f32);
        WindowOptions {
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: "bientot-flyout".into(),
                layer: Layer::Top,
                anchor,
                margin: Some((margin, margin, margin, margin)),
                keyboard_interactivity: keyboard,
                ..Default::default()
            }),
            ..popup_options(None)
        }
    }

    fn kind_name(kind: &WindowKind) -> &'static str {
        match kind {
            WindowKind::Normal => "Normal",
            WindowKind::PopUp => "PopUp",
            WindowKind::LayerShell(_) => "LayerShell",
            _ => "other",
        }
    }

    fn show_bounds(bounds: Bounds<Pixels>) -> String {
        format!(
            "{}x{}+{}+{}",
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y)
        )
    }

    fn x11_window(window: &Window) -> Option<u32> {
        // `Window::window_handle` is GPUI's own handle; the raw one comes from the trait.
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
            RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
            _ => None,
        }
    }

    fn close(reason: &str, window: &mut Window, cx: &mut App) {
        log!("flyout.closed reason={reason}");
        cx.global_mut::<Spike>().flyout = None;
        window.remove_window();
    }

    struct Flyout {
        placement: Placement,
        focus: FocusHandle,
        _activation: Subscription,
    }

    impl Flyout {
        fn new(placement: Placement, window: &mut Window, cx: &mut Context<Self>) -> Self {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let activation = cx.observe_window_activation(window, |_, window, cx| {
                let active = window.is_window_active();
                log!(
                    "flyout.activation active={active} bounds={}",
                    show_bounds(window.bounds())
                );
                if let (Some(probe), Some(xid)) = (&cx.global::<Spike>().x11, x11_window(window)) {
                    probe.report("activation", xid);
                }
                if !active {
                    close("deactivated", window, cx);
                }
            });
            Self {
                placement,
                focus,
                _activation: activation,
            }
        }
    }

    impl Render for Flyout {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .track_focus(&self.focus)
                .size_full()
                .p_4()
                .bg(rgb(0x1f2a44))
                .text_color(rgb(0xffffff))
                .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                    let key = &event.keystroke.key;
                    log!("flyout.key key={key}");
                    if key == "escape" {
                        close("escape", window, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, event: &gpui::MouseDownEvent, _, _| {
                        log!("flyout.mouse_down position={:?}", event.position);
                    }),
                )
                .child(format!("Bientôt spike: {:?}", self.placement))
        }
    }

    /// The tray item, with the identity of `contracts/linux-backends.md`.
    struct SpikeTray {
        sender: UnboundedSender<TrayEvent>,
    }

    impl ksni::Tray for SpikeTray {
        fn id(&self) -> String {
            "dev.bientot.Bientot".into()
        }

        fn title(&self) -> String {
            "Bientôt".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            [16, 22, 24, 32, 48]
                .into_iter()
                .map(|side| ksni::Icon {
                    width: side,
                    height: side,
                    // ARGB32 in network byte order: an opaque blue square.
                    data: [0xff, 0x2f, 0x6f, 0xeb].repeat((side * side) as usize),
                })
                .collect()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "Bientôt spike".into(),
                ..Default::default()
            }
        }

        fn activate(&mut self, x: i32, y: i32) {
            self.sender
                .unbounded_send(TrayEvent::Activate { x, y })
                .ok();
        }

        fn secondary_activate(&mut self, x: i32, y: i32) {
            self.sender
                .unbounded_send(TrayEvent::SecondaryActivate { x, y })
                .ok();
        }

        fn watcher_online(&self) {
            log!("tray.watcher_online");
        }

        fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
            log!("tray.watcher_offline reason={reason:?}");
            true
        }
    }

    /// A second X11 connection, beside GPUI's, that reports and adjusts the flyout's window.
    struct X11Probe {
        connection: RustConnection,
        root: u32,
        atoms: Atoms,
    }

    x11rb::atom_manager! {
        Atoms: AtomsCookie {
            _NET_ACTIVE_WINDOW,
            _NET_WM_STATE,
            _NET_WM_STATE_ABOVE,
            _NET_WM_STATE_SKIP_TASKBAR,
            _NET_WM_STATE_SKIP_PAGER,
            _MOTIF_WM_HINTS,
        }
    }

    impl X11Probe {
        fn connect() -> Result<Self, Box<dyn std::error::Error>> {
            let (connection, screen) = RustConnection::connect(None)?;
            let root = connection.setup().roots[screen].root;
            let atoms = Atoms::new(&connection)?.reply()?;
            Ok(Self {
                connection,
                root,
                atoms,
            })
        }

        /// The window's position on the root window, whether the window manager manages it, and
        /// which window holds the input focus.
        fn report(&self, when: &str, xid: u32) {
            let report = || -> Result<String, Box<dyn std::error::Error>> {
                let connection = &self.connection;
                let geometry = connection.get_geometry(xid)?.reply()?;
                let origin = connection
                    .translate_coordinates(xid, self.root, 0, 0)?
                    .reply()?;
                let attributes = connection.get_window_attributes(xid)?.reply()?;
                let focus = connection.get_input_focus()?.reply()?.focus;
                let active = connection
                    .get_property(
                        false,
                        self.root,
                        self.atoms._NET_ACTIVE_WINDOW,
                        AtomEnum::WINDOW,
                        0,
                        1,
                    )?
                    .reply()?
                    .value32()
                    .and_then(|mut values| values.next());
                let state: Vec<u32> = connection
                    .get_property(false, xid, self.atoms._NET_WM_STATE, AtomEnum::ATOM, 0, 16)?
                    .reply()?
                    .value32()
                    .map(Iterator::collect)
                    .unwrap_or_default();
                let names: Vec<String> = state
                    .iter()
                    .filter_map(|atom| connection.get_atom_name(*atom).ok()?.reply().ok())
                    .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
                    .collect();
                Ok(format!(
                    "x11.report when={when} window={xid:#x} root_xy={},{} size={}x{} override_redirect={} input_focus={focus:#x} focus_is_flyout={} net_active_window={active:#x?} net_wm_state={names:?}",
                    origin.dst_x,
                    origin.dst_y,
                    geometry.width,
                    geometry.height,
                    attributes.override_redirect,
                    focus == xid,
                ))
            };
            match report() {
                Ok(line) => log!("{line}"),
                Err(error) => log!("x11.report_error when={when} error={error}"),
            }
        }

        /// Gives `xid` the input focus, which no window manager does for an override-redirect
        /// window.
        fn focus(&self, xid: u32) {
            let result = self
                .connection
                .set_input_focus(InputFocus::PARENT, xid, x11rb::CURRENT_TIME)
                .map(|cookie| cookie.check());
            self.connection.flush().ok();
            log!("x11.set_input_focus window={xid:#x} result={result:?}");
        }

        /// The fallback of A-1: no decorations (Motif hints), off the taskbar and the pager,
        /// above other windows.
        fn undecorated_above(&self, xid: u32) {
            let connection = &self.connection;
            let no_decorations: [u32; 5] = [1 << 1, 0, 0, 0, 0];
            let motif = connection.change_property32(
                PropMode::REPLACE,
                xid,
                self.atoms._MOTIF_WM_HINTS,
                self.atoms._MOTIF_WM_HINTS,
                &no_decorations,
            );
            let add = |first: u32, second: u32| {
                let message = ClientMessageEvent::new(
                    32,
                    xid,
                    self.atoms._NET_WM_STATE,
                    [1, first, second, 1, 0],
                );
                connection.send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    message,
                )
            };
            let skip = add(
                self.atoms._NET_WM_STATE_SKIP_TASKBAR,
                self.atoms._NET_WM_STATE_SKIP_PAGER,
            );
            let above = add(self.atoms._NET_WM_STATE_ABOVE, 0);
            connection.flush().ok();
            log!(
                "x11.undecorated_above window={xid:#x} motif={:?} skip={:?} above={:?}",
                motif.is_ok(),
                skip.is_ok(),
                above.is_ok()
            );
        }

        fn place(&self, xid: u32, rect: Rect) {
            let result = self.connection.configure_window(
                xid,
                &ConfigureWindowAux::new()
                    .x(rect.x)
                    .y(rect.y)
                    .width(rect.width as u32)
                    .height(rect.height as u32),
            );
            self.connection.flush().ok();
            log!(
                "x11.place window={xid:#x} rect={rect:?} sent={}",
                result.is_ok()
            );
        }
    }
}
