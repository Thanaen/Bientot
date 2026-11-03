//! What the pointer aims at on the open day's strip, outlined with a tooltip over the strip, and
//! the loupe a click on a cluster of short segments opens, where each one is clicked on its own.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Anchor, Bounds, ClickEvent, Context, IntoElement, Pixels, Role, SharedString, Window, anchored,
    canvas, deferred, div, point, prelude::*, px, relative, transparent_black,
};

use super::days::{TICK_TEXT, paint_label, paint_text, segment_quad};
use super::{NextSegment, PreviousSegment, WeekView, button, keystroke};
use crate::flyout;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size, text};
use crate::week::{self, Aimed, LoupeView, SegmentId, SegmentView, Target, WeekState};
use crate::widgets;

impl WeekView {
    /// The pointer moved to `x` over the open day's strip.
    pub(super) fn aim_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let aimed = self
            .strip_bounds
            .get()
            .zip(self.state.as_ref())
            .and_then(|(bounds, state)| {
                let width = f32::from(bounds.size.width);
                let aimed = (width > 0.)
                    .then(|| state.aim(f32::from(x - bounds.origin.x), width))
                    .flatten()?;
                Some((state.focused(), aimed))
            });
        if self.aimed != aimed {
            self.aimed = aimed;
            cx.notify();
        }
    }

    /// The pointer left the open day's strip.
    pub(super) fn stop_aiming(&mut self, cx: &mut Context<Self>) {
        if self.aimed.take().is_some() {
            cx.notify();
        }
    }

    /// A click at `x` on the open day's strip: what it aimed at opens, the popover coming in
    /// under the pointer.
    pub(super) fn click_strip(&mut self, x: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.strip_bounds.get() else {
            return;
        };
        let width = f32::from(bounds.size.width);
        if width <= 0. {
            return;
        }
        self.aimed = None;
        let closed = self
            .state
            .as_ref()
            .is_some_and(|state| state.selection().is_none());
        self.change(window, cx, |state| {
            state.click(f32::from(x - bounds.origin.x), width)
        });
        if let Some(state) = &self.state {
            if state.loupe().is_some() {
                self.loupe_entrance.reset();
            } else if closed && state.selection().is_some() {
                self.bubble_entrance.reset();
            }
        }
    }

    /// What the pointer aims at, shown unless a handle drags, the loupe is open, the open day
    /// changed or it is the segment already selected.
    fn shown_aim(&self, state: &WeekState) -> Option<&Aimed> {
        let (date, aimed) = self.aimed.as_ref()?;
        if self.dragging.is_some() || *date != state.focused() || state.loupe().is_some() {
            return None;
        }
        match aimed.target {
            Target::Segment(id) if state.holder() == Some(id) => None,
            _ => Some(aimed),
        }
    }

    /// The outline of what the pointer aims at on the strip, and its tooltip over it.
    pub(super) fn aim_preview(&self, state: &WeekState, theme: &Theme) -> Option<impl IntoElement> {
        let aimed = self.shown_aim(state)?;
        let width = self.strip_width()?;
        let (x, w) = week::place(aimed.start, aimed.length, width);
        let outlined = w.max(f32::from(size::AIM_OUTLINE_MIN_WIDTH))
            + 2. * f32::from(size::AIM_OUTLINE_OUTSET);
        let centre = x + w / 2.;
        let left = centre - outlined / 2.;
        let (anchor, at) = tooltip_anchor(centre, outlined, width);
        let tooltip = widgets::tooltip_box(theme)
            .child(aimed.text.clone())
            .children(
                aimed
                    .hint
                    .map(|hint| div().text_color(theme.text_muted).child(hint)),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(-BASE_02)
                        .bottom(-BASE_02)
                        .left(px(left))
                        .w(px(outlined))
                        .rounded(radius::SM)
                        .border_1()
                        .border_color(theme.text_muted),
                )
                .child(
                    div().absolute().bottom_full().mb(BASE_04).left(at).child(
                        deferred(
                            anchored()
                                .anchor(anchor)
                                .snap_to_window_with_margin(BASE_08)
                                .child(tooltip),
                        )
                        .with_priority(2),
                    ),
                ),
        )
    }

    /// The loupe under the strip, centred on the hours it shows, which an outline marks on the
    /// strip.
    pub(super) fn loupe(
        &self,
        loupe: LoupeView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let preview = self.loupe_preview(&loupe, theme);
        let inner = px(week::LOUPE_STRIP_WIDTH);
        let outer = inner + size::LOUPE_PADDING * 2.;
        let left = self.strip_width().map_or(px(0.), |width| {
            let centre = (loupe.start + loupe.length / 2.) * width;
            px((centre - f32::from(outer) / 2.)
                .min(width - f32::from(outer))
                .max(0.))
        });
        let close_label = self.text(Key::HistoryLoupeClose);
        let close = button(
            "loupe-close",
            flyout::shortcut_name(close_label, keystroke::CLOSE, self.language),
            theme,
        )
        .h(size::BUTTON_DEFAULT)
        .type_style(text::LABEL_SM)
        .border_color(theme.border_variant)
        .child(close_label)
        .child(flyout::keys(
            flyout::key_caps(keystroke::CLOSE, self.language),
            theme,
        ))
        .on_click(cx.listener(|view, _, window, cx| {
            view.change(window, cx, |state| {
                state.close_bubble();
            })
        }));
        let focused = theme.border_focused;
        let overlays = loupe
            .segments
            .iter()
            .enumerate()
            .map(|(position, segment)| {
                let id = segment.id;
                div()
                    .id(("loupe-segment", position))
                    .role(Role::Button)
                    .aria_label(SharedString::from(segment.accessible_name.clone()))
                    .absolute()
                    .top(-BASE_02)
                    .bottom(-BASE_02)
                    .left(relative(segment.start))
                    .w(relative(segment.length))
                    .min_w(px(week::MINIMUM_SEGMENT_WIDTH))
                    .when_some(self.loupe_focus.get(position), |node, handle| {
                        node.track_focus(handle)
                    })
                    .rounded(radius::XS)
                    .border_2()
                    .border_color(transparent_black())
                    .cursor_pointer()
                    .focus_visible(move |style| style.border_color(focused))
                    // The loupe's strip takes the pointer's clicks.
                    .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                        if event.is_keyboard() {
                            view.pick_in_loupe(id, window, cx);
                        }
                    }))
            })
            .collect::<Vec<_>>();
        let popover = div()
            .id("loupe")
            .role(Role::Dialog)
            .aria_label(SharedString::from(loupe.title.clone()))
            .occlude()
            .w(outer)
            .p(size::LOUPE_PADDING)
            .flex()
            .flex_col()
            .gap(BASE_04)
            .rounded(radius::LG)
            .bg(theme.elevated_surface)
            .border_1()
            .border_color(theme.border)
            .shadow(theme.shadow_elevated())
            .on_action(cx.listener(|view, _: &PreviousSegment, window, cx| {
                view.step_in_loupe(-1, window, cx)
            }))
            .on_action(
                cx.listener(|view, _: &NextSegment, window, cx| view.step_in_loupe(1, window, cx)),
            )
            .child(
                div()
                    .mb(BASE_04)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(BASE_08)
                    .child(
                        div()
                            .whitespace_nowrap()
                            .type_style(text::LABEL_SM)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .font_features(crate::theme::tabular_figures())
                            .child(loupe.title),
                    )
                    .child(close),
            )
            .child(loupe_axis(loupe.ticks, theme))
            .child(
                div()
                    .id("loupe-strip")
                    .relative()
                    .w(inner)
                    .h(size::TIMELINE_ROW_FOCUSED)
                    .cursor_pointer()
                    .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, cx| {
                        view.aim_in_loupe(Some(event.position.x), cx)
                    }))
                    .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                        if !*hovered {
                            view.aim_in_loupe(None, cx);
                        }
                    }))
                    .on_click(cx.listener(|view, event: &ClickEvent, window, cx| {
                        if event.is_keyboard() {
                            return;
                        }
                        view.aim_in_loupe(Some(event.position().x), cx);
                        let picked = view.loupe_aimed.and_then(|position| {
                            view.state
                                .as_ref()?
                                .loupe()?
                                .segments
                                .get(position)
                                .map(|segment| segment.id)
                        });
                        if let Some(id) = picked {
                            view.pick_in_loupe(id, window, cx);
                            // The bubble takes the loupe's place opaque, without a blank frame.
                            view.bubble_entrance.reset_opaque();
                        }
                    }))
                    .child(loupe_strip(
                        loupe.segments.clone(),
                        self.loupe_bounds.clone(),
                        theme,
                    ))
                    .children(overlays)
                    .children(preview),
            );
        div()
            .absolute()
            .inset_0()
            // The hours the loupe shows, on the strip.
            // Drawn around the bar rather than on it, where it shows on any fill.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(loupe.start))
                    .w(relative(loupe.length))
                    .child(
                        div()
                            .absolute()
                            .top(-BASE_04)
                            .bottom(-BASE_04)
                            .left(-BASE_04)
                            .right(-BASE_04)
                            .rounded(radius::MD)
                            .border_2()
                            .border_color(theme.border_focused),
                    ),
            )
            .child(
                div().absolute().top_full().mt(BASE_08).left(left).child(
                    deferred(
                        anchored()
                            .snap_to_window_with_margin(BASE_08)
                            .child(motion::slide_over(self.loupe_time, px(-4.)).apply(popover)),
                    )
                    .with_priority(1),
                ),
            )
    }

    /// The pointer moved to `x` over the loupe's strip, or left it.
    fn aim_in_loupe(&mut self, x: Option<Pixels>, cx: &mut Context<Self>) {
        let aimed = x
            .zip(self.loupe_bounds.get())
            .zip(self.state.as_ref().and_then(WeekState::loupe))
            .and_then(|((x, bounds), loupe)| {
                let spans: Vec<_> = loupe
                    .segments
                    .iter()
                    .map(|segment| (segment.start, segment.length))
                    .collect();
                // Each segment on 24 px at least; no cluster here, the nearest one is taken.
                week::aim(
                    &spans,
                    f32::from(bounds.size.width),
                    f32::from(x - bounds.origin.x),
                )
                .map(|(position, _)| position)
            });
        if self.loupe_aimed != aimed {
            self.loupe_aimed = aimed;
            cx.notify();
        }
    }

    /// The outline of the loupe's segment under the pointer, and its tooltip over the loupe's axis.
    fn loupe_preview(&self, loupe: &LoupeView, theme: &Theme) -> Option<gpui::AnyElement> {
        let position = self.loupe_aimed?;
        let segment = loupe.segments.get(position)?;
        let width = week::LOUPE_STRIP_WIDTH;
        let (x, w) = week::place(segment.start, segment.length, width);
        let outlined = w.max(f32::from(size::AIM_OUTLINE_MIN_WIDTH))
            + 2. * f32::from(size::AIM_OUTLINE_OUTSET);
        let centre = x + w / 2.;
        let (anchor, at) = tooltip_anchor(centre, outlined, width);
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(-BASE_02)
                        .bottom(-BASE_02)
                        .left(px(centre - outlined / 2.))
                        .w(px(outlined))
                        .rounded(radius::SM)
                        .border_1()
                        .border_color(theme.text_muted),
                )
                .child(
                    div().absolute().bottom_full().mb(BASE_04).left(at).child(
                        deferred(
                            anchored()
                                .anchor(anchor)
                                .snap_to_window_with_margin(BASE_08)
                                .child(
                                    widgets::tooltip_box(theme)
                                        .children(loupe.tips.get(position).cloned()),
                                ),
                        )
                        .with_priority(3),
                    ),
                )
                .into_any_element(),
        )
    }

    /// The segment `id` picked in the loupe: the loupe closes on its bubble.
    pub fn pick_in_loupe(&mut self, id: SegmentId, window: &mut Window, cx: &mut Context<Self>) {
        self.change(window, cx, |state| state.open(id));
    }

    /// ← and → in the loupe: the focus goes to the segment before or after.
    fn step_in_loupe(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(count) = self
            .state
            .as_ref()
            .and_then(WeekState::loupe)
            .map(|loupe| loupe.segments.len())
        else {
            return;
        };
        let handles = &self.loupe_focus[..count.min(self.loupe_focus.len())];
        let next = handles
            .iter()
            .position(|handle| handle.is_focused(window))
            .map_or(0, |at| {
                at.saturating_add_signed(delta)
                    .min(handles.len().saturating_sub(1))
            });
        if let Some(handle) = handles.get(next) {
            window.focus(handle, cx);
        }
    }
}

/// Where the tooltip of an outline `outlined` wide about `centre` stands over a strip `width`
/// wide, on its axis rather than on the buttons under it: from the outline's left edge rightwards
/// over the strip's first third, from its right edge leftwards over the last, centred between,
/// so that it stays over the strip.
fn tooltip_anchor(centre: f32, outlined: f32, width: f32) -> (Anchor, Pixels) {
    let (left, right) = (centre - outlined / 2., centre + outlined / 2.);
    if centre < width / 3. {
        (Anchor::BottomLeft, px(left.max(0.)))
    } else if centre > width * 2. / 3. {
        (Anchor::BottomRight, px(right.min(width)))
    } else {
        (Anchor::BottomCenter, px(centre))
    }
}

/// The loupe's segments, painted as the strip's, with their labels. `painted` keeps where.
fn loupe_strip(
    segments: Vec<SegmentView>,
    painted: Rc<Cell<Option<Bounds<Pixels>>>>,
    theme: &Theme,
) -> impl IntoElement {
    let theme = theme.clone();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            painted.set(Some(bounds));
            window.paint_quad(gpui::quad(
                bounds,
                gpui::Corners::all(radius::XS),
                theme.tl_track_focused,
                px(1.),
                theme.border_variant,
                gpui::BorderStyle::Solid,
            ));
            let width = f32::from(bounds.size.width);
            for segment in &segments {
                let (x, w) = week::place(segment.start, segment.length, width);
                let placed = Bounds::new(
                    point(bounds.origin.x + px(x), bounds.origin.y),
                    gpui::size(px(w), bounds.size.height),
                );
                for shape in segment_quad(segment.kind, placed, &theme) {
                    window.paint_quad(shape);
                }
                paint_label(segment, placed, &theme, window, cx);
            }
        },
    )
    .size_full()
}

/// The loupe's axis: each label centred on its tick, within the loupe.
fn loupe_axis(ticks: Vec<(f32, String)>, theme: &Theme) -> impl IntoElement {
    let color = theme.text_muted;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let width = bounds.size.width;
            for (at, label) in &ticks {
                let label_width = widgets::text_width(label, TICK_TEXT, window);
                let x = (width * *at - label_width / 2.)
                    .max(px(0.))
                    .min(width - label_width);
                paint_text(
                    label,
                    point(bounds.origin.x + x, bounds.origin.y),
                    color,
                    window,
                    cx,
                );
            }
        },
    )
    .h(TICK_TEXT.line_height)
    .w_full()
}
