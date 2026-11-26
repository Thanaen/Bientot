use super::*;

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn at(x: i32, y: i32) -> Option<ScreenPoint> {
    Some(ScreenPoint { x, y })
}

/// A 1920 × 1080 laptop on the left, primary, and a 2560 × 1440 monitor on its right.
const MONITORS: [Monitor; 2] = [
    Monitor {
        bounds: Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        primary: true,
    },
    Monitor {
        bounds: Rect {
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
        },
        primary: false,
    },
];

#[test]
fn a_click_on_the_second_monitor_opens_the_flyout_there() {
    assert_eq!(
        clicked_monitor(&MONITORS, at(4400, 1420)),
        Some(MONITORS[1])
    );
}

#[test]
fn an_unknown_click_or_one_off_every_monitor_opens_the_flyout_on_the_primary() {
    assert_eq!(clicked_monitor(&MONITORS, None), Some(MONITORS[0]));
    assert_eq!(clicked_monitor(&MONITORS, at(100, 1300)), Some(MONITORS[0]));
}

#[test]
fn without_a_primary_monitor_the_first_one_holds_the_flyout() {
    let monitors = MONITORS.map(|monitor| Monitor {
        primary: false,
        ..monitor
    });
    assert_eq!(clicked_monitor(&monitors, None), Some(monitors[0]));
    assert_eq!(clicked_monitor(&[], None), None);
}

#[test]
fn the_work_area_leaves_out_a_panel_at_the_bottom() {
    assert_eq!(
        work_area(MONITORS[0].bounds, Some(rect(0, 0, 1920, 1040))),
        rect(0, 0, 1920, 1040)
    );
}

#[test]
fn the_work_area_spanning_two_monitors_is_clipped_to_the_clicked_one() {
    // A panel 40 px tall at the top of the laptop: the window manager publishes one rectangle
    // for both monitors, which starts below the panel on both.
    let net_workarea = Some(rect(0, 40, 4480, 1400));
    assert_eq!(
        work_area(MONITORS[0].bounds, net_workarea),
        rect(0, 40, 1920, 1040)
    );
    assert_eq!(
        work_area(MONITORS[1].bounds, net_workarea),
        rect(1920, 40, 2560, 1400)
    );
}

#[test]
fn a_missing_or_disjoint_work_area_gives_the_whole_monitor() {
    assert_eq!(work_area(MONITORS[1].bounds, None), MONITORS[1].bounds);
    assert_eq!(
        work_area(MONITORS[1].bounds, Some(rect(0, 0, 1920, 1040))),
        MONITORS[1].bounds
    );
}

#[test]
fn the_flyout_goes_to_the_corner_nearest_the_tray_icon() {
    let area = rect(0, 32, 1920, 1048);
    assert_eq!(nearest_corner(area, at(1880, 16)), Corner::TopRight);
    assert_eq!(nearest_corner(area, at(40, 16)), Corner::TopLeft);
    assert_eq!(nearest_corner(area, at(1880, 1070)), Corner::BottomRight);
    assert_eq!(nearest_corner(area, at(40, 1070)), Corner::BottomLeft);
}

#[test]
fn the_corner_is_measured_on_the_clicked_monitor_not_the_desktop() {
    assert_eq!(
        nearest_corner(MONITORS[1].bounds, at(2000, 1420)),
        Corner::BottomLeft
    );
}

#[test]
fn an_unknown_click_opens_the_flyout_top_right() {
    assert_eq!(nearest_corner(MONITORS[0].bounds, None), Corner::TopRight);
}

/// The flyout of a 360 × 400 panel opens in `corner`; the layouts below are those the Lot 4 spike
/// recorded under openbox with xfce4-panel 33 px tall (research.md, « Spike results », A-2 and
/// A-5). T042 records those of real desktops.
fn at_corner(placement: Option<FlyoutPlacement>) -> (Rect, Corner) {
    match placement {
        Some(FlyoutPlacement::At { work_area, corner }) => {
            assert_eq!((work_area.display, work_area.scale), (0, 1.));
            (work_area.bounds, corner)
        }
        other => panic!("a flyout in a work area, not {other:?}"),
    }
}

fn one_screen(net_workarea: Rect) -> Layout {
    Layout {
        monitors: vec![Monitor {
            bounds: rect(0, 0, 1920, 1080),
            primary: false,
        }],
        net_workarea: Some(net_workarea),
    }
}

#[test]
fn a_panel_at_the_bottom_puts_the_flyout_above_it_in_the_bottom_right_corner() {
    let layout = one_screen(rect(0, 0, 1920, 1047));
    assert_eq!(
        at_corner(layout.placement(at(1909, 1064), 0, 1.)),
        (rect(0, 0, 1920, 1047), Corner::BottomRight)
    );
}

#[test]
fn a_panel_at_the_top_puts_the_flyout_below_it_in_the_top_right_corner() {
    let layout = one_screen(rect(0, 33, 1920, 1047));
    assert_eq!(
        at_corner(layout.placement(at(1909, 16), 0, 1.)),
        (rect(0, 33, 1920, 1047), Corner::TopRight)
    );
}

#[test]
fn two_monitors_set_on_one_screen_give_the_clicked_one_and_the_known_limit_on_the_other() {
    // `xrandr --setmonitor` twice on Xvfb, which lists the whole screen as a third monitor and
    // marks none primary; the panel is on the left monitor only.
    let layout = Layout {
        monitors: vec![
            Monitor {
                bounds: rect(0, 0, 960, 1080),
                primary: false,
            },
            Monitor {
                bounds: rect(960, 0, 960, 1080),
                primary: false,
            },
            Monitor {
                bounds: rect(0, 0, 1920, 1080),
                primary: false,
            },
        ],
        net_workarea: Some(rect(0, 0, 1920, 1047)),
    };
    assert_eq!(
        at_corner(layout.placement(at(949, 1064), 0, 1.)),
        (rect(0, 0, 960, 1047), Corner::BottomRight)
    );
    // `_NET_WORKAREA` takes the panel's height off both monitors: 33 px short where no panel is.
    assert_eq!(
        at_corner(layout.placement(at(1900, 1070), 0, 1.)),
        (rect(960, 0, 960, 1047), Corner::BottomRight)
    );
}

#[test]
fn openbox_alone_gives_the_whole_screen_and_an_unknown_click_the_top_right_corner() {
    let layout = one_screen(rect(0, 0, 1920, 1080));
    assert_eq!(
        at_corner(layout.placement(None, 0, 1.)),
        (rect(0, 0, 1920, 1080), Corner::TopRight)
    );
}

#[test]
fn the_work_area_names_the_x_screen_and_the_scale_gpui_draws_at() {
    let placement = one_screen(rect(0, 0, 1920, 1047)).placement(at(10, 10), 1, 1.5);
    let Some(FlyoutPlacement::At { work_area, .. }) = placement else {
        panic!("a flyout in a work area");
    };
    assert_eq!((work_area.display, work_area.scale), (1, 1.5));
}

#[test]
fn without_a_monitor_there_is_no_work_area() {
    let layout = Layout {
        monitors: Vec::new(),
        net_workarea: None,
    };
    assert_eq!(layout.placement(None, 0, 1.), None);
}

#[test]
fn the_x11_scale_is_gpuis_variable_then_xft_dpi_then_one() {
    assert_eq!(x11_scale(Some("2"), Some(144.)), 2.);
    assert_eq!(x11_scale(None, Some(144.)), 1.5);
    assert_eq!(x11_scale(Some("randr"), Some(120.)), 1.25);
    assert_eq!(x11_scale(Some(""), None), 1.);
    assert_eq!(x11_scale(Some("0"), Some(0.)), 1.);
    assert_eq!(x11_scale(None, None), 1.);
}

#[test]
fn the_desktops_are_the_names_of_xdg_current_desktop_lower_cased() {
    assert_eq!(desktops("KDE"), ["kde"]);
    assert_eq!(desktops("ubuntu:GNOME"), ["ubuntu", "gnome"]);
    assert!(desktops("").is_empty());
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// One output of `width` × `height` logical pixels at `x`, `y`, drawn at `scale`.
fn output(x: i32, y: i32, width: i32, height: i32, scale: f32) -> Output {
    Output {
        logical: rect(x, y, width, height),
        physical_height: (height as f32 * scale).round() as i32,
    }
}

#[test]
fn a_click_waybar_gives_on_a_bar_at_the_top_or_the_bottom_anchors_the_flyout_there() {
    // sway's headless output and waybar's positions, research.md, A-5.
    let outputs = [output(0, 0, 1920, 1080, 1.)];
    let sway = names(&["sway"]);
    assert_eq!(
        layer_corner(at(1899, 14), &outputs, &sway),
        Corner::TopRight
    );
    assert_eq!(
        layer_corner(at(1899, 1064), &outputs, &sway),
        Corner::BottomRight
    );
    assert_eq!(
        layer_corner(at(20, 1064), &outputs, &sway),
        Corner::BottomLeft
    );
}

#[test]
fn without_a_click_the_flyout_goes_bottom_right_on_plasma_and_top_right_elsewhere() {
    let outputs = [output(0, 0, 1920, 1080, 1.)];
    assert_eq!(
        layer_corner(None, &outputs, &names(&["kde"])),
        Corner::BottomRight
    );
    assert_eq!(
        layer_corner(None, &outputs, &names(&["sway"])),
        Corner::TopRight
    );
    assert_eq!(layer_corner(None, &[], &[]), Corner::TopRight);
}

#[test]
fn a_click_that_outputs_read_differently_gives_the_default_corner() {
    // A position on the output, from a host that does not say which: on a 1280 × 720 output at
    // 1.5 next to a 1920 × 1080 one, 500, 400 is top left on the first and bottom left on the
    // second.
    let outputs = [
        output(0, 0, 1920, 1080, 1.),
        output(1920, 0, 1280, 720, 1.5),
    ];
    let kde = names(&["kde"]);
    assert_eq!(
        layer_corner(at(500, 400), &outputs, &kde),
        Corner::BottomRight
    );
    assert_eq!(
        layer_corner(at(500, 400), &outputs, &names(&["sway"])),
        Corner::TopRight
    );
    // Only the larger output holds 1900, 1070: its corner.
    assert_eq!(
        layer_corner(at(1900, 1070), &outputs, &names(&["sway"])),
        Corner::BottomRight
    );
    // Both agree on a click near the top left.
    assert_eq!(layer_corner(at(10, 10), &outputs, &kde), Corner::TopLeft);
}

#[test]
fn a_click_off_every_output_gives_the_default_corner() {
    let outputs = [output(0, 0, 1920, 1080, 1.)];
    assert_eq!(
        layer_corner(at(2500, 10), &outputs, &names(&["kde"])),
        Corner::BottomRight
    );
}

#[test]
fn layer_shell_anchors_the_flyout_and_its_absence_leaves_it_to_the_compositor() {
    let outputs = vec![output(0, 0, 1920, 1080, 1.)];
    let layer = WaylandShell {
        layer_shell: true,
        outputs: outputs.clone(),
    };
    assert_eq!(
        layer.placement(at(1899, 14), &names(&["sway"])),
        FlyoutPlacement::Layer {
            corner: Corner::TopRight,
            margin: 12,
        }
    );
    let mutter = WaylandShell {
        layer_shell: false,
        outputs,
    };
    assert_eq!(
        mutter.placement(at(1899, 14), &names(&["gnome"])),
        FlyoutPlacement::CompositorPlaced
    );
}

#[test]
fn the_usable_height_is_the_shortest_outputs_less_a_panel_in_physical_pixels() {
    let shell = |outputs: Vec<Output>| WaylandShell {
        layer_shell: true,
        outputs,
    };
    assert_eq!(
        shell(vec![output(0, 0, 1920, 1080, 1.)]).usable_height(),
        Some(1032)
    );
    // 2560 × 1440 at 125 %: 1152 logical pixels tall, 1104 of them free.
    assert_eq!(
        shell(vec![output(0, 0, 2048, 1152, 1.25)]).usable_height(),
        Some(1380)
    );
    assert_eq!(
        shell(vec![
            output(0, 0, 1920, 1080, 1.),
            output(1920, 0, 1280, 720, 1.5),
        ])
        .usable_height(),
        Some(1008)
    );
    assert_eq!(shell(Vec::new()).usable_height(), None);
}
