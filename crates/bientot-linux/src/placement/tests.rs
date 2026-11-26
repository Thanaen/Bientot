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
