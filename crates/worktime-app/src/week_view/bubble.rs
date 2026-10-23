//! The bubble under the range selected on the open day: what the range holds, De and À, its
//! answers, and the outline with its drag handles over the strip.

use gpui::{
    Context, IntoElement, KeyDownEvent, MouseButton, Role, SharedString, anchored, deferred, div,
    prelude::*, px, relative,
};

use super::{WeekView, button, keystroke, tinted};
use crate::adjust::{TimeField, TimePart};
use crate::flyout;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size, tabular_figures, text};
use crate::week::{self, Edge};
use crate::widgets;

/// De or À as the user is typing it: the part a first digit was typed in waits for the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Typing {
    pub edge: Edge,
    pub field: TimeField,
}

impl WeekView {
    /// The outline of the selection over the strip, the rest of the day dimmed, and a handle at
    /// each edge.
    pub(super) fn selection(
        &self,
        bubble: &week::Bubble,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dim = theme
            .editor
            .opacity(0.55 * motion::ease_out()(self.bubble_time));
        let end = bubble.start + bubble.length;
        let widen = self.strip_width().map_or(0., |width| {
            (f32::from(size::RANGE_MIN_WIDTH) - bubble.length * width).max(0.) / 2.
        });
        let handle = |edge: Edge| {
            // For the pointer only: the keyboard and screen readers move the edges in De and À.
            let time = match edge {
                Edge::Start => bubble.from,
                Edge::End => bubble.to,
            };
            let time = match edge {
                // A range ends at 00:00 only at the midnight closing its day.
                Edge::End if time == jiff::civil::Time::midnight() => "24:00".to_owned(),
                _ => time.strftime("%H:%M").to_string(),
            };
            let dragged = self.dragging == Some(edge);
            div()
                .id(match edge {
                    Edge::Start => "range-start",
                    Edge::End => "range-end",
                })
                .absolute()
                .top(relative(0.5))
                .mt(-size::RANGE_HANDLE_HEIGHT / 2.)
                // The pointer finds it on a wider area than it shows, reaching away from the range
                // so that the two never overlap on a short one. Outside the outline, even past the
                // strip's ends, over the gap beside it.
                .map(|handle| {
                    let offset = -size::RANGE_HANDLE_HIT - px(1.);
                    match edge {
                        Edge::Start => handle.left(offset).justify_end(),
                        Edge::End => handle.right(offset).justify_start(),
                    }
                })
                .w(size::RANGE_HANDLE_HIT)
                .h(size::RANGE_HANDLE_HEIGHT)
                .flex()
                .cursor_ew_resize()
                .child(
                    div()
                        .w(size::RANGE_HANDLE_WIDTH)
                        .h_full()
                        .rounded(px(5.))
                        .border_2()
                        .border_color(theme.text)
                        .bg(theme.editor),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        view.dragging = Some(edge);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .children(dragged.then(|| {
                    div()
                        .absolute()
                        .bottom(size::RANGE_HANDLE_HEIGHT + BASE_08)
                        .left(-BASE_16)
                        .px(BASE_06)
                        .py(BASE_02)
                        .rounded(radius::SM)
                        .bg(theme.text)
                        .text_color(theme.editor)
                        .type_style(text::LABEL_SM)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .font_features(tabular_figures())
                        .whitespace_nowrap()
                        .child(time)
                }))
        };
        let shade = |left: f32, width: f32| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(left))
                .w(relative(width.max(0.)))
                .bg(dim)
        };
        div()
            .absolute()
            .inset_0()
            .child(shade(0., bubble.start))
            .child(shade(end, 1. - end))
            // The outline, inside a halo of the background that sets it off any segment.
            .child(
                div()
                    .absolute()
                    .top(-size::RANGE_OVERHANG)
                    .bottom(-size::RANGE_OVERHANG)
                    .left(relative(bubble.start))
                    .w(relative(bubble.length))
                    // A range too short to see widens about its middle.
                    .ml(-px(widen))
                    .min_w(size::RANGE_MIN_WIDTH)
                    .child(
                        div()
                            .absolute()
                            .top(-BASE_02)
                            .bottom(-BASE_02)
                            .left(-BASE_02)
                            .right(-BASE_02)
                            .rounded(radius::MD + BASE_02)
                            .border_2()
                            .border_color(theme.editor),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(radius::MD)
                            .border_2()
                            .border_color(theme.text),
                    )
                    .when(!bubble.fixed, |outline| {
                        outline.child(handle(Edge::Start)).child(handle(Edge::End))
                    }),
            )
    }

    pub(super) fn bubble(
        &self,
        bubble: week::Bubble,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let start = bubble.start;
        let mut focus_given = false;
        let actions = bubble.actions.iter().enumerate().map(|(position, action)| {
            let label = action.label.clone();
            if action.already {
                return div()
                    .h(size::BUTTON_MEDIUM)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(BASE_04)
                    .whitespace_nowrap()
                    .text_color(theme.text_muted)
                    .child(widgets::icon("check", theme.text_muted).size(size::ICON_SM))
                    .child(flyout::readable(("bubble-already", position), label))
                    .into_any_element();
            }
            if !action.enabled {
                return super::disabled(("bubble-action", position), label.clone(), theme)
                    .border_1()
                    .border_color(theme.border_variant)
                    .child(label)
                    .into_any_element();
            }
            let answer = action.answer;
            let mut control = if action.accent {
                tinted(("bubble-action", position), label.clone().into(), theme)
            } else {
                button(("bubble-action", position), label.clone().into(), theme)
                    .border_color(theme.border_variant)
            };
            if action.enter && !focus_given {
                focus_given = true;
                control = control.track_focus(&self.bubble_focus);
            }
            control
                .child(label)
                .children(action.enter.then(|| {
                    flyout::keys(flyout::key_caps(keystroke::ACCEPT, self.language), theme)
                }))
                .on_click(cx.listener(move |view, _, window, cx| view.answer(answer, window, cx)))
                .into_any_element()
        });
        // A range that takes no answer says why in its notes, without dead buttons.
        let actions: Vec<_> = if bubble.fixed {
            Vec::new()
        } else {
            actions.collect()
        };
        let fields = div()
            .flex()
            .items_end()
            .gap(BASE_08)
            .child(self.range_field(Edge::Start, bubble.from, bubble.fixed, theme, cx))
            .child(self.range_field(Edge::End, bubble.to, bubble.fixed, theme, cx))
            .children(bubble.hint.map(|hint| {
                div()
                    .h(size::BUTTON_MEDIUM)
                    .flex()
                    .items_center()
                    .child(widgets::small(
                        flyout::readable("bubble-hint", hint),
                        theme.text_muted,
                    ))
            }));
        let breakdown = div()
            .flex()
            .flex_wrap()
            .gap_x(BASE_12)
            .gap_y(BASE_04)
            .children(bubble.breakdown.iter().enumerate().map(|(position, held)| {
                div()
                    .flex()
                    .items_center()
                    .gap(BASE_06)
                    .font_features(tabular_figures())
                    .child(super::swatch(Some(held.kind), theme))
                    .child(flyout::readable(
                        ("bubble-held", position),
                        held.name.clone(),
                    ))
                    .children(held.duration.clone().map(|duration| {
                        div().text_color(theme.text_muted).child(flyout::readable(
                            ("bubble-held-duration", position),
                            duration,
                        ))
                    }))
            }));
        let popover = div()
            .id("bubble")
            .role(Role::Dialog)
            .aria_label(SharedString::from(format!(
                "{}, {}",
                bubble.title, bubble.duration
            )))
            .occlude()
            .w(size::POPOVER_WIDTH)
            .flex()
            .flex_col()
            .gap(BASE_12)
            .p(BASE_16)
            .rounded(radius::LG)
            .bg(theme.editor)
            .border_1()
            .border_color(theme.border)
            .shadow(theme.shadow_modal())
            .type_style(text::LABEL_MD)
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(BASE_06)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .gap(BASE_08)
                            .font_features(tabular_figures())
                            .child(div().type_style(text::HEADLINE_SM).child(bubble.title))
                            .child(div().text_color(theme.text_muted).child(bubble.duration)),
                    )
                    .child(breakdown),
            )
            .child(fields)
            .when(!actions.is_empty(), |popover| {
                popover.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(BASE_08)
                        .children(actions),
                )
            })
            .children(
                (!bubble.notes.is_empty() || bubble.preview.is_some()).then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(BASE_04)
                        .children(bubble.notes.iter().enumerate().map(|(position, note)| {
                            widgets::small(
                                flyout::readable(("bubble-note", position), note.clone()),
                                theme.text_muted,
                            )
                        }))
                        .children(bubble.preview.map(|preview| {
                            div().font_features(tabular_figures()).child(widgets::small(
                                flyout::readable("bubble-preview", preview),
                                theme.text_muted,
                            ))
                        }))
                }),
            );
        div()
            .absolute()
            .top_full()
            .mt(BASE_08)
            // Under the range, but never past the strip's end, over the day's totals.
            .map(|anchor| match self.strip_width() {
                Some(width) => anchor.left(px((start * width)
                    .min(width - f32::from(size::POPOVER_WIDTH))
                    .max(0.))),
                None => anchor.left(relative(start.min(0.49))),
            })
            .child(
                deferred(
                    anchored()
                        .snap_to_window_with_margin(BASE_08)
                        .child(motion::slide_over(self.bubble_time, px(-4.)).apply(popover)),
                )
                .with_priority(1),
            )
    }

    /// De or À: hours then minutes, typed or stepped with ↑ and ↓.
    fn range_field(
        &self,
        edge: Edge,
        shown: jiff::civil::Time,
        fixed: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field = match self.typing {
            Some(typing) if typing.edge == edge => typing.field,
            // A range ends at 00:00 only at the midnight closing its day.
            _ if edge == Edge::End && shown == jiff::civil::Time::midnight() => {
                TimeField::new(24, 0)
            }
            _ => TimeField::new(shown.hour(), shown.minute()),
        };
        let (header, handles) = match edge {
            Edge::Start => (Key::HistoryBubbleFromHeader, &self.from_fields),
            Edge::End => (Key::HistoryBubbleToHeader, &self.to_fields),
        };
        let header = self.text(header);
        // Screen readers name the field by the edge it moves, as its handle.
        let edge_name = self.text(match edge {
            Edge::Start => Key::HistoryRangeStartHandle,
            Edge::End => Key::HistoryRangeEndHandle,
        });
        let part = |part: TimePart, name: Key| {
            if fixed {
                return div().child(field.part_text(part)).into_any_element();
            }
            let handle = match part {
                TimePart::Hours => &handles.0,
                TimePart::Minutes => &handles.1,
            };
            let (accent, accent_background) = (theme.accent, theme.accent_background);
            div()
                .id(match (edge, part) {
                    (Edge::Start, TimePart::Hours) => "from-hours",
                    (Edge::Start, TimePart::Minutes) => "from-minutes",
                    (Edge::End, TimePart::Hours) => "to-hours",
                    (Edge::End, TimePart::Minutes) => "to-minutes",
                })
                .role(Role::SpinButton)
                .aria_label(SharedString::from(format!(
                    "{edge_name}, {}",
                    self.text(name)
                )))
                .aria_numeric_value(f64::from(field.value(part)))
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(match (edge, part) {
                    (Edge::End, TimePart::Hours) => 24.,
                    (_, TimePart::Hours) => 23.,
                    (_, TimePart::Minutes) => 59.,
                })
                .rounded(radius::XS)
                .track_focus(handle)
                .focus(move |style| style.bg(accent_background).text_color(accent))
                .on_key_down(cx.listener(move |view, event, window, cx| {
                    view.range_key(edge, part, event, window, cx)
                }))
                .child(field.part_text(part))
                .into_any_element()
        };
        let hours = handles.0.clone();
        div()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .child(widgets::small(header, theme.text_muted))
            .child(
                div()
                    .id(match edge {
                        Edge::Start => "from",
                        Edge::End => "to",
                    })
                    .role(Role::Group)
                    .aria_label(edge_name)
                    .key_context(super::FIELD_CONTEXT)
                    .h(size::BUTTON_MEDIUM)
                    .w(px(72.))
                    .px(BASE_08)
                    .flex()
                    .items_center()
                    .rounded(radius::SM)
                    .font_features(tabular_figures())
                    .map(|group| {
                        if fixed {
                            group.bg(theme.surface).text_color(theme.text_muted)
                        } else {
                            group
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.editor)
                                .cursor_text()
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    widgets::focus_unless_taken(&hours, window, cx)
                                })
                        }
                    })
                    .child(part(
                        TimePart::Hours,
                        Key::TimeFieldHoursAutomationPropertiesName,
                    ))
                    .child(":")
                    .child(part(
                        TimePart::Minutes,
                        Key::TimeFieldMinutesAutomationPropertiesName,
                    )),
            )
    }

    /// De or À half typed takes what it shows, so that an answer records the range shown.
    pub(super) fn apply_typing(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if let Some(Typing { edge, field }) = self.typing.take() {
            self.apply(edge, field, window, cx);
        }
    }

    /// Moves `edge` to the time `field` shows; 24 hours is 24:00, the midnight closing the day,
    /// whatever minutes were typed after it.
    fn apply(
        &mut self,
        edge: Edge,
        field: TimeField,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let time = match field.value(TimePart::Hours) {
            24 => jiff::civil::Time::midnight(),
            _ => field.time(),
        };
        self.change(window, cx, |state| state.set_time(edge, time));
    }

    /// A key in De or À: a digit types the part, ↑ and ↓ step it. The range moves once the part
    /// is complete, then shows where it snapped.
    fn range_key(
        &mut self,
        edge: Edge,
        part: TimePart,
        event: &KeyDownEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        // What was half typed in the other edge's field takes effect before this one changes.
        if self.typing.is_some_and(|typing| typing.edge != edge) {
            self.apply_typing(window, cx);
        }
        let Some(bubble) = self.state.as_ref().and_then(|state| state.bubble()) else {
            return;
        };
        let shown = match edge {
            Edge::Start => bubble.from,
            Edge::End => bubble.to,
        };
        let mut field = match self.typing {
            Some(typing) if typing.edge == edge => typing.field,
            // As the field shows it: the midnight closing the day is 24:00.
            _ if edge == Edge::End && shown == jiff::civil::Time::midnight() => {
                TimeField::new(24, 0)
            }
            _ => TimeField::new(shown.hour(), shown.minute()),
        };
        let complete = match event.keystroke.key.as_str() {
            "up" | "down" => {
                field = stepped(field, edge, part, event.keystroke.key == "up");
                true
            }
            // Entrée gives the answer the bubble shows; a time half typed that moves the edge
            // moves it first, so that the bubble shows what an answer would record.
            "enter" => {
                cx.stop_propagation();
                match bubble.entered() {
                    Some(answer) => self.answer(answer, window, cx),
                    None => self.apply_typing(window, cx),
                }
                return;
            }
            // Between the parts, not to the next segment: the range typed would be lost.
            "left" | "right" => {
                let (hours, minutes) = match edge {
                    Edge::Start => &self.from_fields,
                    Edge::End => &self.to_fields,
                };
                let to = if event.keystroke.key == "left" {
                    hours
                } else {
                    minutes
                };
                window.focus(&to.clone(), cx);
                cx.stop_propagation();
                return;
            }
            key => {
                let Some(digit) = key.parse::<u8>().ok().filter(|digit| *digit <= 9) else {
                    // A letter typed here is no shortcut of the week; Entrée and Échap still are.
                    if key.chars().count() == 1 {
                        cx.stop_propagation();
                    }
                    return;
                };
                // 24:00 has no minutes to type.
                if part == TimePart::Minutes && field.value(TimePart::Hours) == 24 {
                    cx.stop_propagation();
                    return;
                }
                let last = match (edge, part) {
                    (Edge::End, TimePart::Hours) => 24,
                    (_, TimePart::Hours) => 23,
                    (_, TimePart::Minutes) => 59,
                };
                let complete = field.type_digit_up_to(part, digit, last);
                if complete && part == TimePart::Hours {
                    let minutes = match edge {
                        Edge::Start => self.from_fields.1.clone(),
                        Edge::End => self.to_fields.1.clone(),
                    };
                    window.focus(&minutes, cx);
                }
                complete
            }
        };
        if complete {
            self.typing = None;
            self.apply(edge, field, window, cx);
        } else {
            self.typing = Some(Typing { edge, field });
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// ↑ or ↓ on a part of De or À: a minute or an hour later or earlier, carried into the hours and
/// held within the day, from 00:00 to 23:59 for De and to 24:00 for À. Wrapping round the clock
/// would throw the edge to the other end of the day.
fn stepped(field: TimeField, edge: Edge, part: TimePart, up: bool) -> TimeField {
    let step = match part {
        TimePart::Hours => 60,
        TimePart::Minutes => 1,
    };
    let last = match edge {
        Edge::Start => 23 * 60 + 59,
        Edge::End => 24 * 60,
    };
    let minutes =
        i16::from(field.value(TimePart::Hours)) * 60 + i16::from(field.value(TimePart::Minutes));
    let minutes = (minutes + if up { step } else { -step }).clamp(0, last);
    TimeField::new((minutes / 60) as i8, (minutes % 60) as i8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(edge: Edge, part: TimePart, up: bool, (hours, minutes): (i8, i8)) -> String {
        stepped(TimeField::new(hours, minutes), edge, part, up).text()
    }

    #[test]
    fn arrows_carry_into_the_hours_and_stop_at_the_ends_of_the_day() {
        assert_eq!(step(Edge::End, TimePart::Minutes, true, (9, 59)), "10:00");
        assert_eq!(step(Edge::End, TimePart::Hours, true, (23, 30)), "24:00");
        assert_eq!(step(Edge::End, TimePart::Hours, true, (24, 0)), "24:00");
        assert_eq!(step(Edge::End, TimePart::Minutes, false, (24, 0)), "23:59");
        assert_eq!(step(Edge::Start, TimePart::Hours, true, (23, 30)), "23:59");
        assert_eq!(step(Edge::Start, TimePart::Hours, false, (0, 30)), "00:00");
    }

    #[test]
    fn the_end_types_24_00() {
        let mut field = TimeField::new(16, 45);
        assert!(!field.type_digit_up_to(TimePart::Hours, 2, 24));
        assert!(field.type_digit_up_to(TimePart::Hours, 4, 24));
        assert_eq!(field.text(), "24:00");
    }
}
