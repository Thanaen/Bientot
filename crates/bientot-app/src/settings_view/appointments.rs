//! « Rendez-vous réguliers » under the groups of « Comptabilisation »: the appointments in a
//! card, each with its switch and « Modifier », then the editor or « Ajouter un rendez-vous ».

use gpui::{
    AnyElement, Context, Focusable as _, FontWeight, Role, SharedString, Window, div, prelude::*,
};

use super::{
    RowText, SettingsView, card, field_error, filled, outlined, row_frame, switch_frame, time_field,
};
use bientot_domain::configuration::{AppointmentProposal, DismissedProposal};

use crate::settings::appointments::{AppointmentRow, AppointmentsView, EditorView};
use crate::settings::{Clock, RowTone};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, text};

impl SettingsView {
    pub(super) fn appointments_section(
        &mut self,
        view: AppointmentsView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = view.rows.len();
        let rows: Vec<AnyElement> = view
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| self.appointment_row(row, index + 1 == count, theme, cx))
            .collect();
        let list = card(theme).children(rows).children(view.empty.map(|empty| {
            div()
                .py(BASE_12)
                .type_style(text::LABEL_SM)
                .text_color(theme.text_muted)
                .child(empty)
        }));
        let below = match view.editor {
            Some(editor) => self.appointment_editor(editor, theme, cx),
            None => div()
                .children(view.add.map(|add| {
                    outlined("add-appointment", add, token::BUTTON_MEDIUM, theme).on_click(
                        cx.listener(|view, _, window, cx| view.open_appointment(None, window, cx)),
                    )
                }))
                .into_any_element(),
        };
        div()
            .id("appointments")
            .flex()
            .flex_col()
            .gap(BASE_08)
            .child(
                div()
                    .id("appointments-heading")
                    .role(Role::Heading)
                    .type_style(text::HEADLINE_SM)
                    .child(view.heading),
            )
            .child(
                div()
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .child(view.description),
            )
            .child(list)
            .children(view.callout.map(|callout| self.callout(callout, theme, cx)))
            .child(div().flex().child(below))
            .child(
                card(theme)
                    .mt(BASE_12)
                    .child(self.row(view.proposals, true, 0., theme, cx)),
            )
            .children(
                view.proposals_callout
                    .map(|callout| self.callout(callout, theme, cx)),
            )
            .into_any_element()
    }

    fn appointment_row(
        &mut self,
        row: AppointmentRow,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (toggled, edited) = (row.id.clone(), row.id.clone());
        let switch_id = SharedString::from(format!("appointment-switch-{}", row.id));
        let edit_name = row.edit_name;
        let control = div()
            .flex()
            .items_center()
            .gap(BASE_12)
            .children(row.edit.map(|edit| {
                outlined(
                    SharedString::from(format!("edit-appointment-{edited}")),
                    edit,
                    token::BUTTON_MEDIUM,
                    theme,
                )
                .aria_label(edit_name)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.open_appointment(Some(&edited), window, cx)
                }))
            }))
            .child(
                switch_frame(switch_id, row.title.clone(), row.enabled, true, theme).on_click(
                    cx.listener(move |view, _, window, cx| {
                        let pending = view.form.toggle_appointment(&toggled);
                        view.submit(pending, window, cx);
                    }),
                ),
            )
            .into_any_element();
        row_frame(
            RowText {
                title: row.title.into(),
                description: row.description.into(),
                tone: if row.enabled {
                    RowTone::Normal
                } else {
                    RowTone::Disabled
                },
                link: None,
                status: None,
                error: None,
            },
            control,
            None,
            last,
            theme,
        )
        .into_any_element()
    }

    fn appointment_editor(
        &mut self,
        editor: EditorView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let placeholder = editor.name_placeholder.clone();
        self.appointment_name
            .update(cx, |input, _| input.set_placeholder(placeholder));
        let label = |text: &'static str| {
            div()
                .font_weight(FontWeight::MEDIUM)
                .type_style(text::LABEL_SM)
                .child(text)
        };
        let field = |name: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(BASE_04)
                .child(label(name))
                .child(control)
        };
        let days = div()
            .id("appointment-days")
            .role(Role::RadioGroup)
            .aria_label(editor.day_label)
            .flex()
            .gap(BASE_04)
            .children(editor.days.into_iter().map(|day| {
                let weekday = day.weekday;
                choice_button(
                    ("appointment-day", weekday.to_monday_one_offset() as usize),
                    day.short,
                    day.full,
                    day.chosen,
                    Some(DAY_WIDTH),
                    theme,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.form.choose_appointment_day(weekday);
                    cx.notify();
                }))
            }));
        let repeats = div()
            .id("appointment-repeat")
            .role(Role::RadioGroup)
            .aria_label(editor.repeat_label)
            .flex()
            .gap(BASE_04)
            .children(editor.repeats.map(|repeat| {
                let alternate = repeat.alternate;
                choice_button(
                    ("appointment-repeat", usize::from(alternate)),
                    repeat.label,
                    repeat.label,
                    repeat.chosen,
                    None,
                    theme,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.form.choose_appointment_repeat(alternate);
                    cx.notify();
                }))
            }));
        let modes = div()
            .id("appointment-mode")
            .role(Role::RadioGroup)
            .aria_label(editor.mode_label)
            .flex()
            .gap(BASE_04)
            .children(editor.modes.map(|mode| {
                let automatic = mode.automatic;
                choice_button(
                    ("appointment-mode", usize::from(automatic)),
                    mode.label,
                    mode.label,
                    mode.chosen,
                    None,
                    theme,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.form.choose_appointment_mode(automatic);
                    cx.notify();
                }))
            }));
        let next = editor.next.map(|next| {
            let weeks = div()
                .id("appointment-next")
                .role(Role::RadioGroup)
                .aria_label(next.label)
                .flex()
                .gap(BASE_04)
                .children(next.weeks.map(|week| {
                    let later = week.later;
                    choice_button(
                        ("appointment-week", usize::from(later)),
                        week.label,
                        week.full,
                        week.chosen,
                        None,
                        theme,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.form.choose_appointment_week(later);
                        cx.notify();
                    }))
                }));
            field(next.label, weeks.into_any_element())
        });
        let invalid = editor.hours_error.is_some();
        let hours = div()
            .id("appointment-hours")
            .role(Role::Group)
            .aria_label(editor.hours_label)
            .flex()
            .items_center()
            .gap(BASE_08)
            .child(self.appointment_clock(Clock::AppointmentStart, false, theme, cx))
            .child(div().text_color(theme.text_muted).child("–"))
            .child(self.appointment_clock(Clock::AppointmentEnd, invalid, theme, cx));
        let save = filled("save-appointment", editor.save, token::BUTTON_MEDIUM, theme)
            .on_click(cx.listener(|view, _, window, cx| view.save_appointment(window, cx)));
        let footer = div()
            .pt(BASE_04)
            .flex()
            .items_center()
            .gap(BASE_08)
            .children(editor.delete.map(|delete| {
                outlined("delete-appointment", delete, token::BUTTON_MEDIUM, theme)
                    .text_color(theme.error)
                    .on_click(cx.listener(|view, _, window, cx| {
                        let pending = view.form.delete_appointment();
                        view.submit(pending, window, cx);
                    }))
            }))
            .child(div().flex_1())
            .child(
                outlined(
                    "cancel-appointment",
                    editor.cancel,
                    token::BUTTON_MEDIUM,
                    theme,
                )
                .on_click(cx.listener(|view, _, window, cx| {
                    let pending = view.form.close_appointment();
                    view.submit(pending, window, cx);
                })),
            )
            .child(save);
        card(theme)
            .id("appointment-editor")
            .role(Role::Group)
            .aria_label(editor.title)
            .w_full()
            .py(BASE_16)
            .flex()
            .flex_col()
            .gap(BASE_16)
            .child(div().font_weight(FontWeight::MEDIUM).child(editor.title))
            .child(field(
                editor.name_label,
                div()
                    .w(super::SELECT_WIDTH)
                    .child(self.appointment_name.clone())
                    .into_any_element(),
            ))
            .child(field(editor.day_label, days.into_any_element()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(BASE_24)
                    .child(field(editor.repeat_label, repeats.into_any_element()))
                    .children(next),
            )
            .child(field(
                editor.hours_label,
                div()
                    .flex()
                    .flex_col()
                    .child(hours)
                    .children(editor.hours_error.map(|error| field_error(error, theme)))
                    .into_any_element(),
            ))
            .child(field(
                editor.mode_label,
                div()
                    .flex()
                    .flex_col()
                    .gap(BASE_08)
                    .child(modes)
                    .child(
                        div()
                            .type_style(text::LABEL_SM)
                            .text_color(theme.text_muted)
                            .child(editor.mode_note),
                    )
                    .into_any_element(),
            ))
            .child(
                div()
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .child(editor.note),
            )
            .child(footer)
            .into_any_element()
    }

    fn appointment_clock(
        &self,
        clock: Clock,
        invalid: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        time_field(
            clock,
            self.form.time(clock),
            &self.clocks[clock as usize],
            self.focused_clock == Some(clock),
            invalid,
            self.language,
            theme,
            cx,
            Self::time_key,
        )
    }

    /// Opens the editor on the appointment `id`, or on a new one, its name focused.
    pub fn open_appointment(
        &mut self,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.form.open_appointment(id) else {
            return;
        };
        self.appointment_name
            .update(cx, |input, cx| input.set_text(&name, cx));
        self.scroll_to_appointments();
        let focus = self.appointment_name.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Opens the editor on `proposal`, see [`SettingsForm::open_proposal`]; `false` while the
    /// dialog holds the page.
    pub fn open_proposal(
        &mut self,
        proposal: &AppointmentProposal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.commit_focused(window, cx);
        self.language_open = false;
        cx.notify();
        if !self.form.open_proposal(proposal) {
            window.focus(&self.safe_focus, cx);
            return false;
        }
        let name = self
            .form
            .appointment_draft()
            .map(|draft| draft.name.clone());
        if let Some(name) = name {
            self.appointment_name
                .update(cx, |input, cx| input.set_text(&name, cx));
            let focus = self.appointment_name.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        self.scroll_to_appointments();
        true
    }

    /// « Ne plus proposer pour les jeudis »; `false` while the dialog holds the page.
    pub(super) fn dismiss_proposal(
        &mut self,
        dismissal: DismissedProposal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.form.dialog().is_some() {
            return false;
        }
        let pending = self.form.dismiss_proposal(dismissal);
        self.submit(pending, window, cx);
        true
    }

    /// « Compter automatiquement à l’avenir »; `false` while the dialog holds the page.
    pub(super) fn count_automatically(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.form.dialog().is_some() {
            return false;
        }
        let pending = self.form.count_automatically(id);
        let accepted = pending.is_some();
        self.submit(pending, window, cx);
        accepted
    }

    /// Brings « Rendez-vous réguliers » to the top of the page, which follows the other groups.
    pub(super) fn scroll_to_appointments(&self) {
        let page = self.form.view();
        let before = usize::from(page.source.is_some()) + page.sections.len();
        self.content_scroll.scroll_to_top_of_item(before);
    }

    pub(super) fn save_appointment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.appointment_name.read(cx).text().to_owned();
        let pending = self.form.save_appointment(&name);
        self.submit(pending, window, cx);
    }
}

/// Wide enough for « Ven. », so that the days line up.
const DAY_WIDTH: gpui::Pixels = gpui::px(52.);

/// A choice of the editor, as a radio button: `width` lines the days up, the other choices
/// fit their label.
fn choice_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    name: impl Into<SharedString>,
    chosen: bool,
    width: Option<gpui::Pixels>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (hover, focused) = (theme.element_hover, theme.border_focused);
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(name)
        .aria_toggled(if chosen {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .h(token::BUTTON_MEDIUM)
        .map(|button| match width {
            Some(width) => button.w(width),
            None => button.px(BASE_12),
        })
        .flex()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .rounded(radius::SM)
        .border_1()
        .tab_index(0)
        .cursor_pointer()
        .focus_visible(move |style| style.border_color(focused))
        .map(|button| {
            if chosen {
                button
                    .border_color(theme.accent)
                    .bg(theme.accent_background)
                    .text_color(theme.accent)
                    .font_weight(FontWeight::MEDIUM)
            } else {
                button
                    .border_color(theme.border)
                    .bg(theme.editor)
                    .hover(move |style| style.bg(hover))
            }
        })
        .child(label.into())
}
