//! « Détails de l’activité et du crédit »: its toggle under the open day, and the panel it opens
//! under that toggle with the day's credit and corrections.

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, FontWeight, Pixels, Role, SharedString,
    Window, div, prelude::*,
};
use jiff::SignedDuration;
use worktime_domain::accounting::AdjustmentId;

use super::{Layout, WeekView, button, disabled, row_with_trailing, value_color};
use crate::adjust::window::Opener;
use crate::adjust_view;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyle, TypeStyled as _, radius, size, tabular_figures, text};
use crate::week::{Credit, Details, DetailsAction, WeekState};
use crate::widgets;

/// The widest value the credit's column is sized for: a debt under 100 hours, in the format of
/// [`Credit::value_text`].
const WIDEST_CREDIT: SignedDuration = SignedDuration::from_mins(-(99 * 60 + 59));

/// The credit's values, in tabular figures so that they line up.
const CREDIT_VALUE: TypeStyle = TypeStyle {
    tabular: true,
    ..text::LABEL_SM
};

impl WeekView {
    /// The toggle, on a day with a summary to detail.
    pub(super) fn details_toggle(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.state.as_ref().filter(|state| state.has_details())?;
        let label = self.text(Key::HistoryDetailsToggleContent);
        let toggle = widgets::disclosure("details-toggle", label, state.details_open(), theme)
            .on_click(cx.listener(|view, _, _, cx| view.toggle_details(cx)));
        Some(toggle.into_any_element())
    }

    /// Opening the panel scrolls the open day into view.
    pub fn toggle_details(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &mut self.state else {
            return;
        };
        self.reveal_details = state.toggle_details();
        cx.notify();
    }

    fn edit_listed(&mut self, id: &AdjustmentId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workday) = self.state.as_ref().and_then(|state| state.edit_listed(id)) {
            let _ = self.open_edit(workday, id.clone(), window, cx);
        }
    }

    fn add_listed(&mut self, cx: &mut Context<Self>) {
        if let Some((workday, seed)) = self.state.as_ref().and_then(WeekState::add_listed) {
            let _ = adjust_view::window::open(Some(workday), seed, Opener::Week, cx);
        }
    }

    /// The width of the credit's labels and of its values: the longest of each, whatever the
    /// day, so that the corrections beside them stay put from one day to the next.
    fn credit_widths(&self, window: &Window) -> (Pixels, Pixels) {
        let labels = [
            Key::HistoryEarnedLabelText,
            Key::HistoryAppliedLabelText,
            Key::HistoryAvailableLabelText,
            Key::HistoryDebtLabelText,
            Key::HistoryExpiredLabelText,
        ]
        .map(|key| widgets::text_width(self.text(key), text::LABEL_SM, window));
        let label = labels.into_iter().fold(Pixels::ZERO, Pixels::max);
        let value = widgets::text_width(
            &Credit::value_text(WIDEST_CREDIT, self.language),
            CREDIT_VALUE,
            window,
        );
        (label.ceil(), value.ceil())
    }

    pub(super) fn details(
        &self,
        details: Details,
        layout: Layout,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (label_width, value_width) = self.credit_widths(window);
        let credit = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .type_style(text::LABEL_SM)
            .whitespace_nowrap()
            .child(widgets::caps(details.credit_title, theme).mb(BASE_02))
            .children(
                details
                    .credit
                    .into_iter()
                    .enumerate()
                    .map(|(position, line)| {
                        div()
                            .id(("credit", position))
                            .role(Role::Group)
                            .aria_label(SharedString::from(format!(
                                "{}, {}",
                                line.label, line.value
                            )))
                            .flex()
                            .gap(BASE_12)
                            .child(
                                div()
                                    .w(label_width)
                                    .text_color(theme.text_muted)
                                    .child(line.label),
                            )
                            .child(
                                div()
                                    .w(value_width)
                                    .flex()
                                    .justify_end()
                                    .type_style(CREDIT_VALUE)
                                    .text_color(value_color(line.tone, theme))
                                    .child(line.value),
                            )
                    }),
            );
        let corrections = if details.corrections.is_empty() {
            div()
                .type_style(text::LABEL_SM)
                .text_color(theme.text_muted)
                .child(self.text(Key::HistoryNoManualAdjustmentsText))
        } else {
            div().flex().flex_col().gap(BASE_02).children(
                details
                    .corrections
                    .into_iter()
                    .enumerate()
                    .map(|(position, row)| {
                        let id = row.id;
                        let edit = action(
                            ("edit-listed", position),
                            row.edit,
                            row.edit_name.into(),
                            None,
                            theme,
                            cx.listener(move |view, _, window, cx| {
                                view.edit_listed(&id, window, cx)
                            }),
                        );
                        let disposition = div()
                            .flex_none()
                            .font_weight(FontWeight::MEDIUM)
                            .child(row.disposition);
                        // Without `min_w_0` the range keeps its width, so in a narrow window the
                        // duration and « Modifier » wrap below it instead of painting over it.
                        let ranges = div()
                            .flex_1()
                            .flex()
                            .flex_wrap()
                            .gap_x(BASE_08)
                            .font_features(tabular_figures())
                            .text_color(theme.text_muted)
                            .children(row.ranges);
                        let line = row_with_trailing(
                            [disposition.into_any_element(), ranges.into_any_element()],
                            row.duration,
                            edit,
                            theme,
                        );
                        div()
                            .pl(BASE_08)
                            .pr(BASE_04)
                            .flex()
                            .flex_col()
                            .rounded(radius::SM)
                            .bg(theme.element)
                            .child(line)
                            .children(row.note.map(|note| {
                                div()
                                    .pb(BASE_06)
                                    .type_style(text::LABEL_SM)
                                    .text_color(theme.text_muted)
                                    .child(note)
                            }))
                    }),
            )
        };
        let add = action(
            "add-listed",
            details.add,
            details.add.label.into(),
            Some("pencil"),
            theme,
            cx.listener(|view, _, _, cx| view.add_listed(cx)),
        );
        div()
            .id("details")
            .role(Role::Group)
            .aria_label(SharedString::from(details.accessible_name))
            .flex()
            .map(|panel| match layout {
                Layout::Wide => panel.items_start(),
                Layout::Narrow => panel.flex_col(),
            })
            .gap(BASE_16)
            .p(BASE_12)
            .rounded(radius::MD)
            .border_1()
            .border_color(theme.border_variant)
            .bg(theme.editor)
            .child(credit)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(BASE_06)
                    .child(widgets::caps(details.corrections_title, theme))
                    .child(corrections)
                    .child(div().flex().child(add)),
            )
    }
}

/// « Modifier » or « Ajouter une correction », named `name`: a button while it can act, else
/// the same box drawn disabled, so that the rows keep their columns.
fn action(
    id: impl Into<ElementId>,
    action: DetailsAction,
    name: SharedString,
    icon: Option<&'static str>,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let (frame, color) = if action.enabled {
        let frame = button(id, name, theme).bg(theme.element).on_click(on_click);
        (frame, theme.text_muted)
    } else {
        (disabled(id, name, theme).border_1(), theme.text_placeholder)
    };
    frame
        .border_color(theme.border_variant)
        .children(icon.map(|icon| widgets::icon(icon, color).size(size::ICON_SM)))
        .child(action.label)
        .into_any_element()
}
