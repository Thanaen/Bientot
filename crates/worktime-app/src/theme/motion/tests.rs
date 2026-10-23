use gpui::px;

use super::*;

/// Reference values solved exactly with rational arithmetic, apart from this module.
fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "{actual} is not {expected}"
    );
}

#[test]
fn every_curve_starts_and_ends_exactly_at_rest() {
    for curve in [
        Box::new(ease_out()) as Box<dyn Fn(f32) -> f32>,
        Box::new(ease_in_out()),
        Box::new(ease_drawer()),
    ] {
        assert_eq!(curve(0.), 0.);
        assert_eq!(curve(1.), 1.);
    }
}

#[test]
fn the_curves_follow_their_control_points() {
    let ease_out = ease_out();
    assert_close(ease_out(0.25), 0.775382);
    assert_close(ease_out(0.5), 0.965983);
    let ease_in_out = ease_in_out();
    assert_close(ease_in_out(0.25), 0.052889);
    assert_close(ease_in_out(0.5), 0.595971);
    let ease_drawer = ease_drawer();
    assert_close(ease_drawer(0.25), 0.779131);
    assert_close(ease_drawer(0.5), 0.954809);
}

#[test]
fn css_ease_reaches_its_published_midpoint() {
    assert_close(cubic_bezier(0.25, 0.1, 0.25, 1.)(0.5), 0.802403);
}

#[test]
fn control_points_on_the_diagonal_give_linear_progress() {
    let linear = cubic_bezier(1. / 3., 1. / 3., 2. / 3., 2. / 3.);
    for x in [0.1, 0.3, 0.5, 0.9] {
        assert_close(linear(x), x);
    }
}

#[test]
fn a_slowdown_stretches_a_duration() {
    assert_eq!(
        scaled(Duration::from_millis(200), 4.),
        Duration::from_millis(800)
    );
    assert_eq!(
        scaled(Duration::from_millis(200), 0.5),
        Duration::from_millis(100)
    );
    assert_eq!(scaled(Duration::from_millis(200), 1e300), Duration::MAX);
}

#[cfg(debug_assertions)]
#[test]
fn the_slowdown_is_a_finite_factor_above_zero() {
    assert_eq!(parse_slowdown("4"), Some(4.));
    assert_eq!(parse_slowdown(" 2.5 "), Some(2.5));
    assert_eq!(parse_slowdown("0.5"), Some(0.5));
    assert_eq!(parse_slowdown("1e9"), Some(100.));
    assert_eq!(parse_slowdown("1e-9"), Some(0.01));
    for invalid in ["", "fast", "0", "-2", "inf", "NaN", "4x"] {
        assert_eq!(parse_slowdown(invalid), None, "{invalid:?}");
    }
}

#[test]
fn without_a_slowdown_the_tokens_keep_their_budgets() {
    if std::env::var_os("WORKTIME_MOTION_SLOWDOWN").is_some() {
        return;
    }
    assert_eq!(PRESS.duration(), Duration::from_millis(140));
    assert_eq!(TOOLTIP.duration(), Duration::from_millis(150));
    assert_eq!(MENU.duration(), Duration::from_millis(200));
    assert_eq!(DIALOG.duration(), Duration::from_millis(250));
}

#[test]
fn an_entrance_runs_from_its_first_frame_to_rest() {
    let duration = Duration::from_millis(250);
    let at = |millis| share(Duration::from_millis(millis), duration, false);
    assert_eq!(at(0), Some(0.));
    assert_close(at(25).unwrap(), 0.1);
    assert_close(at(200).unwrap(), 0.8);
    let running = [at(20), at(125), at(240)].map(Option::unwrap);
    assert_close(running[1], 0.5);
    assert!(running.iter().all(|time| 0. < *time && *time < 1.));
    assert!(running.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn an_entrance_at_rest_asks_for_no_more_frames() {
    let duration = Duration::from_millis(250);
    for elapsed in [250, 251, 10_000] {
        assert_eq!(share(Duration::from_millis(elapsed), duration, false), None);
    }
    assert_eq!(share(Duration::ZERO, Duration::ZERO, false), None);
}

#[test]
fn with_reduced_motion_an_entrance_is_at_rest_at_once() {
    let duration = Duration::from_millis(250);
    for elapsed in [0, 125, 250] {
        assert_eq!(share(Duration::from_millis(elapsed), duration, true), None);
    }
}

#[test]
fn a_slide_comes_in_hidden_from_its_offset_and_ends_in_place() {
    assert_eq!(
        slide_in(0., px(4.)),
        Entering {
            opacity: 0.,
            offset: px(4.),
        }
    );
    let quarter = slide_in(0.25, px(4.));
    assert_close(quarter.opacity, 0.775382);
    assert_close(f32::from(quarter.offset), 0.898473);
    let midway = slide_in(0.5, px(-4.));
    assert_close(midway.opacity, 0.965983);
    assert_close(f32::from(midway.offset), -0.136068);
    assert_eq!(
        slide_in(1., px(4.)),
        Entering {
            opacity: 1.,
            offset: px(0.),
        }
    );
}

#[test]
fn over_other_content_a_slide_hides_the_page_from_its_first_frames_and_moves_as_in_the_flow() {
    assert_eq!(
        slide_over(0., px(-4.)),
        Entering {
            opacity: 0.,
            offset: px(-4.),
        }
    );
    let early = slide_over(0.05, px(6.));
    assert_close(early.opacity, 0.965983);
    assert_close(f32::from(early.offset), 4.741385);
    assert_eq!(slide_over(OPAQUE_AT, px(6.)).opacity, 1.);
    let midway = slide_over(0.5, px(6.));
    assert_eq!(midway.opacity, 1.);
    assert_close(f32::from(midway.offset), 0.204102);
    assert_eq!(
        slide_over(1., px(-4.)),
        Entering {
            opacity: 1.,
            offset: px(0.),
        }
    );
}

#[test]
fn a_pose_is_drawn_shifted_in_place_and_faded() {
    let mut element = Entering {
        opacity: 0.25,
        offset: px(3.),
    }
    .apply(gpui::div());
    let style = element.style();
    assert_eq!(style.position, Some(gpui::Position::Relative));
    assert_eq!(style.inset.top, Some(px(3.).into()));
    assert_eq!(style.opacity, Some(0.25));
}
