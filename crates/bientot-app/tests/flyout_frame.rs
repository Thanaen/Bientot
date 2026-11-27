//! Where no system frames the flyout's window, as on Linux, the app draws the frame: a rounded
//! panel with room around it for its shadow when a compositor blends the transparent window, a
//! square panel that fills it when none does. Where the system frames it, as on Windows, the panel
//! fills the window. The shadow itself shows in the screenshots only (`flyout-frame-drawn`), since
//! these windows draw nothing to read back. It needs the headless GPUI of the `screenshots`
//! feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::rc::Rc;

use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout;
use bientot_app::host::{self, Source, Today};
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_app::theme::Theme;
use bientot_core::demo::DemoScenario;
use bientot_platform::placement::{FlyoutPlacement, Frame, InMemoryDesktop};
use bientot_platform::tray::TrayHost;
use gpui::{Bounds, Hsla, Pixels, Size, px};

/// What the open flyout's window shows of its frame, in logical pixels.
struct Shown {
    window: Size<Pixels>,
    /// The panel: the largest bordered quad.
    panel: Bounds<Pixels>,
    panel_radius: Pixels,
    /// The background of a quad that covers the whole window, under the panel.
    behind: Option<Hsla>,
}

fn opened(frame: Frame, placement: FlyoutPlacement) -> Shown {
    let mut cx = common::app(true, flyout::bind_keys);
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let today = Today::open(Source::Demo(Box::new(host)), Language::French).unwrap();
    let tray = match placement {
        FlyoutPlacement::Window => TrayHost::Absent,
        _ => TrayHost::Present,
    };
    cx.update(|cx| {
        host::install(today, cx);
        let desktop = InMemoryDesktop {
            placement,
            usable_height: None,
            frame,
        };
        shell::install(Rc::new(desktop), tray, cx);
        shell::open_flyout(cx);
    });
    cx.run_until_parked();
    let window = cx.update(|cx| cx.windows())[0];
    cx.update_window(window, |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| {
        let scale = window.scale_factor();
        let quads = window.painted_quads();
        let size = window.bounds().size;
        let panel = quads
            .iter()
            .filter(|quad| quad.border_widths.top.0 > 0.)
            .max_by(|a, b| {
                let area =
                    |quad: &&gpui::Quad| quad.bounds.size.width.0 * quad.bounds.size.height.0;
                area(a).total_cmp(&area(b))
            })
            .expect("the panel's border");
        let behind = quads
            .iter()
            .find(|quad| {
                quad.bounds.size.width.0 / scale >= f32::from(size.width)
                    && quad.bounds.size.height.0 / scale >= f32::from(size.height)
            })
            .and_then(|quad| quad.background.as_solid());
        let logical = |pixels: f32| px(pixels / scale);
        Shown {
            window: size,
            panel: Bounds::new(
                gpui::point(
                    logical(panel.bounds.origin.x.0),
                    logical(panel.bounds.origin.y.0),
                ),
                gpui::size(
                    logical(panel.bounds.size.width.0),
                    logical(panel.bounds.size.height.0),
                ),
            ),
            panel_radius: logical(panel.corner_radii.top_left.0),
            behind,
        }
    })
    .unwrap()
}

#[test]
fn with_a_compositor_the_rounded_panel_leaves_room_for_its_shadow_in_the_window() {
    let shown = opened(Frame::App, FlyoutPlacement::CompositorPlaced);
    assert_eq!(shown.panel.origin, gpui::point(px(12.), px(12.)));
    assert_eq!(shown.window.width, shown.panel.size.width + px(24.));
    assert_eq!(shown.window.height, shown.panel.size.height + px(24.));
    assert_eq!(shown.panel_radius, px(8.));
    assert_eq!(shown.behind, None, "the window is clear around the panel");
}

#[test]
fn without_a_compositing_manager_the_square_panel_fills_the_window() {
    let shown = opened(Frame::Square, FlyoutPlacement::CompositorPlaced);
    assert_eq!(shown.panel.origin, gpui::point(px(0.), px(0.)));
    assert_eq!(shown.window, shown.panel.size);
    assert_eq!(shown.panel_radius, px(0.));
}

#[test]
fn in_window_mode_the_square_panel_fills_the_window_even_with_a_compositor() {
    let shown = opened(Frame::App, FlyoutPlacement::Window);
    assert_eq!(shown.window, shown.panel.size);
    assert_eq!(shown.panel_radius, px(0.));
}

#[test]
fn where_the_system_frames_the_window_the_rounded_panel_fills_it_over_the_border_colour() {
    let shown = opened(Frame::System, FlyoutPlacement::CompositorPlaced);
    assert_eq!(shown.panel.origin, gpui::point(px(0.), px(0.)));
    assert_eq!(shown.window, shown.panel.size);
    assert_eq!(shown.panel_radius, px(8.));
    assert_eq!(shown.behind, Some(Theme::light().border));
}
