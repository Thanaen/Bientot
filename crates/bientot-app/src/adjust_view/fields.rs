//! The fields of « Corriger le temps… »: what to correct, the times and the classification.

use bientot_domain::adjustments::ManualClassification;
use gpui::{
    Context, FocusHandle, FontWeight, MouseButton, Role, Toggled, div, prelude::*, px,
    transparent_black,
};

use super::{AdjustView, Bound, label};
use crate::adjust::{AdjustPage, TimePart, classification_label};
use crate::flyout;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, tabular_figures, text};
use crate::widgets;

impl AdjustView {
    pub(super) fn intents(
        &self,
        page: &AdjustPage,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let locked = page.locked;
        let options = page.intents.iter().enumerate().map(|(position, option)| {
            let intent = option.intent;
            let (ring, dot) = match (locked, option.selected) {
                (true, _) => (theme.border, theme.text_placeholder),
                (false, true) => (theme.accent, theme.accent),
                (false, false) => (theme.text_placeholder, transparent_black()),
            };
            let focused = theme.border_focused;
            let hover = theme.element_hover;
            let item = div()
                .id(("intent", position))
                .role(Role::RadioButton)
                .aria_label(option.label)
                .aria_toggled(if option.selected {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .h(token::BUTTON_MEDIUM)
                .px(BASE_08)
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(radius::SM)
                .border_1()
                .border_color(transparent_black())
                .text_color(if locked {
                    theme.text_placeholder
                } else {
                    theme.text
                })
                .child(
                    div()
                        .size(token::ICON_SM)
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(ring)
                        .bg(if locked { theme.element } else { theme.editor })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(option.selected, |radio| {
                            radio.child(div().size(px(6.)).rounded_full().bg(dot))
                        }),
                )
                .child(option.label);
            if locked {
                return item;
            }
            item.tab_index(0)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .focus_visible(move |style| style.border_color(focused))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.change(cx, |form| form.set_intent(intent))
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(BASE_06)
            .child(label(page.intent_header, theme))
            .child(
                div()
                    .id("intents")
                    .role(Role::RadioGroup)
                    .aria_label(page.intent_header)
                    .flex()
                    .flex_col()
                    .gap(BASE_02)
                    .mx(-BASE_08)
                    .children(options),
            )
    }

    fn time_field(
        &self,
        bound: Bound,
        page: &AdjustPage,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (header, field, focus) = match bound {
            Bound::Start => (Key::AdjustTimeStartHeader, self.form.start, &self.start),
            Bound::End => (Key::AdjustTimeEndHeader, self.form.end, &self.end),
        };
        let header = self.text(header);
        let locked = page.locked;
        let invalid = page.interval_error.is_some();
        let segment = |part: TimePart, handle: &FocusHandle, name: Key| {
            let id = match (bound, part) {
                (Bound::Start, TimePart::Hours) => "start-hours",
                (Bound::Start, TimePart::Minutes) => "start-minutes",
                (Bound::End, TimePart::Hours) => "end-hours",
                (Bound::End, TimePart::Minutes) => "end-minutes",
            };
            let (accent, accent_background) = (theme.accent, theme.accent_background);
            let segment = div()
                .id(id)
                .role(Role::SpinButton)
                .aria_label(self.text(name))
                .aria_numeric_value(f64::from(field.value(part)))
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(match part {
                    TimePart::Hours => 23.,
                    TimePart::Minutes => 59.,
                })
                .rounded(radius::XS)
                .child(field.part_text(part));
            if locked {
                return segment;
            }
            segment
                .track_focus(handle)
                .focus(move |style| style.bg(accent_background).text_color(accent))
                .on_key_down(cx.listener(move |view, event, window, cx| {
                    view.time_key(bound, part, event, window, cx)
                }))
        };
        let hours = focus.hours.clone();
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .child(label(header, theme))
            .child(
                div()
                    .id(match bound {
                        Bound::Start => "start",
                        Bound::End => "end",
                    })
                    .role(Role::Group)
                    .aria_label(header)
                    .h(token::BUTTON_MEDIUM)
                    .px(BASE_08)
                    .flex()
                    .items_center()
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(if invalid { theme.error } else { theme.border })
                    .bg(if locked { theme.element } else { theme.editor })
                    .text_color(if locked {
                        theme.text_placeholder
                    } else {
                        theme.text
                    })
                    .font_features(tabular_figures())
                    .when(!locked, |field| {
                        field
                            .cursor_text()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                widgets::focus_unless_taken(&hours, window, cx)
                            })
                    })
                    .child(segment(
                        TimePart::Hours,
                        &focus.hours,
                        Key::TimeFieldHoursAutomationPropertiesName,
                    ))
                    .child(":")
                    .child(segment(
                        TimePart::Minutes,
                        &focus.minutes,
                        Key::TimeFieldMinutesAutomationPropertiesName,
                    )),
            )
    }

    pub(super) fn times(
        &self,
        page: &AdjustPage,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let fields = if page.local_times.is_some() {
            div()
                .flex()
                .gap(BASE_12)
                .child(self.time_field(Bound::Start, page, theme, cx))
                .child(self.time_field(Bound::End, page, theme, cx))
        } else {
            div()
                .flex()
                .gap(BASE_12)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(BASE_04)
                        .child(label(self.text(Key::AdjustTimeDurationHeader), theme))
                        .child(self.duration.clone()),
                )
                .child(div().flex_1())
        };
        let error = page
            .interval_error
            .map(|message| inline_error("interval-error", message, theme))
            .or_else(|| {
                page.duration_error
                    .map(|message| inline_error("duration-error", message, theme))
            });
        let zone = page
            .local_times
            .clone()
            .map(|zone| widgets::small(flyout::readable("adjust-zone", zone), theme.text_muted));
        div()
            .flex()
            .flex_col()
            .gap(BASE_06)
            .child(fields)
            .children(error)
            .children(zone)
    }

    pub(super) fn classification(
        &self,
        page: &AdjustPage,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let header = self.text(Key::AdjustTimeClassificationHeader);
        let locked = page.locked;
        let options = [
            ManualClassification::Credited,
            ManualClassification::Excluded,
        ]
        .into_iter()
        .enumerate()
        .map(|(position, classification)| {
            let name = classification_label(classification, self.language);
            let selected = self.form.classification() == classification;
            let (background, border, color) = match (locked, selected) {
                (false, true) => (theme.accent_background, theme.accent, theme.accent),
                (true, true) => (theme.element_active, theme.border, theme.text_placeholder),
                (false, false) => (transparent_black(), transparent_black(), theme.text_muted),
                (true, false) => (
                    transparent_black(),
                    transparent_black(),
                    theme.text_placeholder,
                ),
            };
            let focused = theme.border_focused;
            let option = div()
                .id(("classification", position))
                .role(Role::RadioButton)
                .aria_label(name)
                .aria_toggled(if selected {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius::SM)
                .border_1()
                .border_color(border)
                .bg(background)
                .text_color(color)
                .when(selected, |option| option.font_weight(FontWeight::MEDIUM))
                .whitespace_nowrap()
                .overflow_hidden()
                .child(name);
            if locked {
                return option;
            }
            option
                .tab_index(0)
                .cursor_pointer()
                .focus_visible(move |style| style.border_color(focused))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.change(cx, |form| form.set_classification(classification))
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .child(label(header, theme))
            .child(
                div()
                    .id("classification")
                    .role(Role::RadioGroup)
                    .aria_label(header)
                    .h(token::BUTTON_MEDIUM)
                    .p(BASE_02)
                    .flex()
                    .gap(BASE_02)
                    .rounded(radius::MD)
                    .border_1()
                    .border_color(theme.border_variant)
                    .bg(theme.element)
                    .children(options),
            )
    }
}

fn inline_error(id: &'static str, message: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Alert)
        .aria_label(message)
        .flex()
        .items_start()
        .gap(BASE_06)
        .type_style(text::LABEL_SM)
        .text_color(theme.error)
        .child(
            div()
                .h(text::LABEL_SM.line_height)
                .flex()
                .items_center()
                .child(widgets::icon("circle-alert", theme.error).size(token::ICON_SM)),
        )
        .child(div().flex_1().min_w_0().child(message))
}
