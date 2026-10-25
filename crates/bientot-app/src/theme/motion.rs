//! How long animations last and how they ease, for `Animation::new(MENU.duration())
//! .with_easing(ease_out())`. The durations sit in the middle of Emil Kowalski's budgets.
//!
//! `with_animation` and `with_spring` already show their rest state at once while
//! `App::reduce_motion` is set. An [`Entrance`] does the same.

use std::time::{Duration, Instant};

use gpui::{App, Pixels, Styled, Window};

/// The length of one kind of animation. A debug build stretches it by
/// `BIENTOT_MOTION_SLOWDOWN`, to judge an animation by feel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing(Duration);

pub const PRESS: Timing = Timing(Duration::from_millis(140));
pub const TOOLTIP: Timing = Timing(Duration::from_millis(150));
pub const MENU: Timing = Timing(Duration::from_millis(200));
pub const DIALOG: Timing = Timing(Duration::from_millis(250));

impl Timing {
    pub fn duration(self) -> Duration {
        scaled(self.0, slowdown())
    }
}

#[cfg(debug_assertions)]
fn slowdown() -> f64 {
    static SLOWDOWN: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SLOWDOWN.get_or_init(|| {
        std::env::var("BIENTOT_MOTION_SLOWDOWN")
            .ok()
            .and_then(|value| parse_slowdown(&value))
            .unwrap_or(1.)
    })
}

#[cfg(not(debug_assertions))]
fn slowdown() -> f64 {
    1.
}

/// The factor `BIENTOT_MOTION_SLOWDOWN` asks for: a finite number above zero, held within
/// 0.01..=100 so a typo neither freezes the motion nor skips it.
#[cfg(debug_assertions)]
fn parse_slowdown(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|factor| factor.is_finite() && *factor > 0.)
        .map(|factor| factor.clamp(0.01, 100.))
}

fn scaled(duration: Duration, factor: f64) -> Duration {
    Duration::try_from_secs_f64(duration.as_secs_f64() * factor).unwrap_or(Duration::MAX)
}

/// How far something that shows has come in, timed from its first frame on the executor's
/// clock. Not `with_animation`, which times itself on the wall clock: the executor's clock is the
/// one a test can step. What comes in takes input from its first frame.
#[derive(Debug, Default)]
pub struct Entrance(Phase);

#[derive(Clone, Copy, Debug, Default)]
enum Phase {
    #[default]
    Hidden,
    Coming(Instant),
    /// Stays so until `reset`: turning reduced motion off while it shows does not replay it.
    AtRest,
}

impl Entrance {
    /// For what shows from the start rather than coming in: it comes in after the next `reset`.
    pub fn at_rest() -> Self {
        Self(Phase::AtRest)
    }

    /// The share of `timing` gone since the first frame this was called for, from 0 to 1 at
    /// rest, asking for the next frame until then. Linear: one entrance can drive several
    /// curves.
    pub fn time(&mut self, timing: Timing, window: &mut Window, cx: &App) -> f32 {
        let now = cx.background_executor().now();
        let shown = match self.0 {
            Phase::Hidden => now,
            Phase::Coming(shown) => shown,
            Phase::AtRest => return 1.,
        };
        match share(
            now.saturating_duration_since(shown),
            timing.duration(),
            cx.reduce_motion(),
        ) {
            Some(time) => {
                self.0 = Phase::Coming(shown);
                window.request_animation_frame();
                time
            }
            None => {
                self.0 = Phase::AtRest;
                1.
            }
        }
    }

    /// For what no longer shows: it comes in again the next time.
    pub fn reset(&mut self) {
        self.0 = Phase::Hidden;
    }
}

/// The share of `duration` gone after `elapsed` while it runs, or `None` at rest: from
/// `duration` on, and at once with reduced motion.
fn share(elapsed: Duration, duration: Duration, reduce_motion: bool) -> Option<f32> {
    (!reduce_motion && elapsed < duration).then(|| elapsed.as_secs_f32() / duration.as_secs_f32())
}

/// The share of an entrance by which what comes in over other content is opaque. Until then
/// what lies beneath shows through it, and its text collides with the text beneath. A 60 Hz
/// frame is 8 % of a menu's entrance and 7 % of a dialog's, so the first frame that shows them
/// must already hide the page.
pub const OPAQUE_AT: f32 = 0.1;

/// Where something coming in stands on one frame: how opaque, and how far from its place.
#[derive(Debug, PartialEq)]
pub struct Entering {
    pub opacity: f32,
    pub offset: Pixels,
}

impl Entering {
    /// Draws `element` at this pose: shifted down by `offset` without moving its neighbours,
    /// and faded to `opacity`.
    pub fn apply<E: Styled>(self, element: E) -> E {
        element.relative().top(self.offset).opacity(self.opacity)
    }
}

/// What comes in within the flow at `time`, a share of its entrance, on the strong ease-out: it
/// fades in over the whole entrance while it slides `from` an offset into place.
pub fn slide_in(time: f32, from: Pixels) -> Entering {
    entering(ease_out()(time), time, from)
}

/// What comes in over other content at `time`: it slides as [`slide_in`] does but is opaque by
/// [`OPAQUE_AT`].
pub fn slide_over(time: f32, from: Pixels) -> Entering {
    entering(ease_out()((time / OPAQUE_AT).min(1.)), time, from)
}

fn entering(opacity: f32, time: f32, from: Pixels) -> Entering {
    Entering {
        opacity,
        offset: from * (1. - ease_out()(time)),
    }
}

/// Strong ease-out, for what enters or leaves.
pub fn ease_out() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.23, 1., 0.32, 1.)
}

/// Strong ease-in-out, for what moves on screen.
pub fn ease_in_out() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.77, 0., 0.175, 1.)
}

/// The iOS drawer curve.
pub fn ease_drawer() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.32, 0.72, 0., 1.)
}

/// CSS `cubic-bezier(x1, y1, x2, y2)`: the curve from (0, 0) to (1, 1) through those two
/// control points, read as progress for a fraction of the time. As in CSS, `x1` and `x2` lie
/// within 0..=1: outside, time could run backwards and the search below would not hold.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    debug_assert!(
        (0. ..=1.).contains(&x1) && (0. ..=1.).contains(&x2),
        "cubic-bezier x control points lie within 0..=1"
    );
    let axis = |p1: f32, p2: f32, t: f32| {
        let u = 1. - t;
        3. * u * u * t * p1 + 3. * u * t * t * p2 + t * t * t
    };
    move |x| {
        // Exact ends, so the last frame and a reduced-motion frame land precisely at rest.
        if x <= 0. {
            return 0.;
        }
        if x >= 1. {
            return 1.;
        }
        // The x axis rises monotonically: 24 halvings find its parameter far below a pixel of
        // any frame.
        let (mut low, mut high) = (0_f32, 1_f32);
        for _ in 0..24 {
            let middle = (low + high) / 2.;
            if axis(x1, x2, middle) < x {
                low = middle;
            } else {
                high = middle;
            }
        }
        axis(y1, y2, (low + high) / 2.)
    }
}

#[cfg(test)]
mod tests;
