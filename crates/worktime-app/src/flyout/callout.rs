//! The flyout's source hint and the outlined buttons of its callouts.

use gpui::{Action, AnyElement, FontWeight, Role, SharedString, div, prelude::*};

use super::{
    CountAutomatically, CountUnrecorded, Flyout, HideCount, HideTrustOffer, OpenGeneralSettings,
    Refresh, ResolveReview, ShowUnrecorded, UndoCount, UseBuiltInDetection, control, control_icon,
    key_caps, keys, keystroke, readable,
};
use crate::settings_view::outlined_on_tint;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size, text};
use crate::time_text::minutes_text;
use crate::tray_model::{ReviewAction, ReviewCommand, appointment_name};
use crate::widgets::{Hue, callout_box_with_icon};
use crate::workday::{FailureRemedy, SourceHint};

impl Flyout {
    /// What the source reported, and what to do about it: a failure is a warning, with a refresh
    /// to try again; unrecorded time is only information, with « Oui, compter N min » to count
    /// it as work, filled as the usual answer, on a day that can be corrected, and « Voir sur la
    /// frise » to settle it in the week.
    pub(super) fn source_callout(&self, theme: &Theme) -> Option<AnyElement> {
        let hint = self.day.source_hint.as_ref()?;
        let (failure, remedies) = match hint {
            SourceHint::Failure {
                remedy: FailureRemedy::UseBuiltIn,
                ..
            } => (true, vec![self.use_built_in_button(theme)]),
            SourceHint::Failure {
                remedy: FailureRemedy::OpenSettings,
                ..
            } => (
                true,
                vec![self.open_settings_button("open-source-settings", theme)],
            ),
            SourceHint::Unrecorded {
                workday,
                intervals,
                duration,
                ..
            } => {
                // Left out on a day off rather than dimmed, as the day's commands are; « Voir
                // sur la frise » still settles the time in the week.
                let count = self.tracking.can_correct_time().then(|| {
                    let id = "count-unrecorded";
                    let label = self.language.format(
                        Key::FlyoutCountUnrecordedFormat,
                        &[&minutes_text(*duration, self.language)],
                    );
                    let count = CountUnrecorded {
                        workday: workday.clone(),
                        intervals: intervals.clone(),
                    };
                    self.reveal_on_tab(
                        id,
                        filled_button(id, label, theme).on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(count.clone()), cx)
                        }),
                    )
                });
                let show = self.callout_button(
                    "show-unrecorded",
                    self.text(Key::FlyoutShowUnrecorded).into(),
                    None,
                    Box::new(ShowUnrecorded {
                        date: workday.date,
                        intervals: intervals.clone(),
                    }),
                    theme,
                );
                (false, count.into_iter().chain([show]).collect())
            }
        };
        let (hue, icon_name) = if failure {
            (Hue::Warning, "activity")
        } else {
            (Hue::Info, Hue::Info.icon())
        };
        Some(
            callout_box_with_icon(hue, icon_name, theme)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(BASE_02)
                        .child(readable("source-body", hint.message().clone()))
                        .child(
                            div()
                                .pt(BASE_06)
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(BASE_08)
                                .children(remedies)
                                .when(failure, |row| {
                                    row.child(self.refresh_button("refresh", theme))
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The absence in a recurring appointment that awaits an answer: what it was, and the two
    /// answers of the week's bubble.
    pub(super) fn appointment_callout(&self, theme: &Theme) -> Option<AnyElement> {
        let review = self.day.appointment_review.as_ref()?;
        let language = self.language;
        let title = appointment_name(
            review.appointment.as_deref().unwrap_or_default(),
            review.workday.date.weekday(),
            language,
        );
        let clock = |time| language.short_time(time);
        let range = language.format(
            Key::FlyoutAppointmentReviewRangeFormat,
            &[
                &format!("{}–{}", clock(review.local_start), clock(review.local_end)),
                &minutes_text(review.duration, language),
            ],
        );
        // Stacked across the callout, as the long answer would not share a row in French; the
        // first one, the usual answer, in the accent.
        let answer = |id, label, action, usual: bool| {
            let command = ReviewCommand {
                review: review.clone(),
                action,
            };
            let button = outlined_on_tint(id, self.text(label), size::BUTTON_MEDIUM, theme)
                .w_full()
                .justify_center()
                .when(usual, |button| {
                    button
                        .border_color(theme.accent.opacity(0.5))
                        .text_color(theme.accent)
                        .font_weight(FontWeight::MEDIUM)
                })
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(ResolveReview(command.clone())), cx)
                });
            self.reveal_on_tab(id, button)
        };
        Some(
            callout_box_with_icon(Hue::Info, "repeat", theme)
                .id("appointment-review")
                .role(Role::Group)
                .aria_label(SharedString::from(title.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(BASE_02)
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(
                            div()
                                .type_style(text::LABEL_SM)
                                .text_color(theme.text_muted)
                                .child(range),
                        )
                        .child(div().pt(BASE_04).child(readable(
                            "appointment-review-body",
                            self.text(Key::FlyoutAppointmentReviewText),
                        )))
                        .child(
                            div()
                                .pt(BASE_06)
                                .flex()
                                .flex_col()
                                .gap(BASE_06)
                                .child(answer(
                                    "appointment-confirm",
                                    Key::HistoryBubbleConfirmContent,
                                    ReviewAction::ConfirmWork,
                                    true,
                                ))
                                .child(answer(
                                    "appointment-exclude",
                                    Key::HistoryBubbleReclassifyContent,
                                    ReviewAction::Exclude,
                                    false,
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    /// An absence of an appointment counted without asking (B): what was counted, with
    /// « Ce n’était pas du travail » to take it back and a cross that only closes the callout.
    pub(super) fn counted_callout(&self, theme: &Theme) -> Option<AnyElement> {
        let counted = self.day.counted_absence.as_ref()?;
        let language = self.language;
        let title = appointment_name(
            &counted.appointment,
            counted.workday.date.weekday(),
            language,
        );
        let clock = |time| language.short_time(time);
        let range = language.format(
            Key::FlyoutAppointmentReviewRangeFormat,
            &[
                &format!(
                    "{}–{}",
                    clock(counted.local_start),
                    clock(counted.local_end)
                ),
                &minutes_text(counted.duration, language),
            ],
        );
        let undo = UndoCount(counted.adjustment_id.clone());
        let id = "counted-undo";
        let button = outlined_on_tint(
            id,
            self.text(Key::HistoryBubbleReclassifyContent),
            size::BUTTON_MEDIUM,
            theme,
        )
        .w_full()
        .justify_center()
        .on_click(move |_, window, cx| window.dispatch_action(Box::new(undo.clone()), cx));
        let hide = HideCount(counted.adjustment_id.clone());
        Some(self.closable_callout(
            ["counted-absence", "counted-body", "counted-close"],
            Hue::Success,
            title,
            range,
            self.text(Key::FlyoutAppointmentCountedText).into(),
            self.reveal_on_tab(id, button),
            Box::new(hide),
            theme,
        ))
    }

    /// « Compter automatiquement à l’avenir », after the user confirmed the appointment's
    /// absences the same way several times in a row.
    pub(super) fn trust_callout(&self, theme: &Theme) -> Option<AnyElement> {
        let offer = self.day.trust_offer.as_ref()?;
        let language = self.language;
        let title = appointment_name(&offer.appointment, offer.weekday, language);
        let accept = CountAutomatically(offer.appointment_id.clone());
        let id = "trust-accept";
        let button = outlined_on_tint(
            id,
            self.text(Key::FlyoutAppointmentTrustAction),
            size::BUTTON_MEDIUM,
            theme,
        )
        .w_full()
        .justify_center()
        .border_color(theme.accent.opacity(0.5))
        .text_color(theme.accent)
        .font_weight(FontWeight::MEDIUM)
        .on_click(move |_, window, cx| window.dispatch_action(Box::new(accept.clone()), cx));
        Some(
            self.closable_callout(
                ["trust-offer", "trust-body", "trust-close"],
                Hue::Info,
                title,
                String::new(),
                language
                    .format(Key::FlyoutAppointmentTrustFormat, &[&offer.streak])
                    .into(),
                self.reveal_on_tab(id, button),
                Box::new(HideTrustOffer(offer.appointment_id.clone())),
                theme,
            ),
        )
    }

    /// A callout of an appointment with its name, a muted line when `detail` is not empty, a
    /// sentence, one button and a cross dispatching `close`; `ids` names the callout, its
    /// sentence and its cross.
    #[allow(clippy::too_many_arguments)]
    fn closable_callout(
        &self,
        [id, body_id, close_id]: [&'static str; 3],
        hue: Hue,
        title: String,
        detail: String,
        body: SharedString,
        button: AnyElement,
        close: Box<dyn Action>,
        theme: &Theme,
    ) -> AnyElement {
        let cross = control(
            close_id,
            self.text(Key::CommandErrorDialogCloseButton).into(),
            theme,
        )
        .size(size::BUTTON_MEDIUM)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(control_icon(close_id, "x", theme))
        .on_click(move |_, window, cx| window.dispatch_action(close.boxed_clone(), cx));
        callout_box_with_icon(hue, "repeat", theme)
            .id(id)
            .role(Role::Group)
            .aria_label(SharedString::from(title.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(BASE_02)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(BASE_08)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                            // No taller than the title: the cross overflows the row.
                            .child(
                                div()
                                    .h_0()
                                    .flex()
                                    .items_center()
                                    .child(self.reveal_on_tab(close_id, cross)),
                            ),
                    )
                    .when(!detail.is_empty(), |column| {
                        column.child(
                            div()
                                .type_style(text::LABEL_SM)
                                .text_color(theme.text_muted)
                                .child(detail),
                        )
                    })
                    .child(div().pt(BASE_04).child(readable(body_id, body)))
                    .child(div().pt(BASE_06).child(button)),
            )
            .into_any_element()
    }

    /// « Utiliser la détection intégrée », the one filled button of the callout.
    fn use_built_in_button(&self, theme: &Theme) -> AnyElement {
        let id = "use-built-in";
        let button = filled_button(
            id,
            self.text(Key::SettingsActivityWatchCalloutAction),
            theme,
        )
        .on_click(|_, window, cx| window.dispatch_action(Box::new(UseBuiltInDetection), cx));
        self.reveal_on_tab(id, button)
    }

    /// An outlined button of a callout, filled as « Réessayer » of the update callout is, with
    /// the key caps of `keystroke` when it runs a command that has one. A click dispatches
    /// `action`.
    pub(super) fn callout_button(
        &self,
        id: &'static str,
        label: SharedString,
        keystroke: Option<&str>,
        action: Box<dyn Action>,
        theme: &Theme,
    ) -> AnyElement {
        let name = match keystroke {
            Some(keystroke) => super::shortcut_name(&label, keystroke, self.language),
            None => label.clone(),
        };
        let button = outlined_on_tint(id, label, size::BUTTON_MEDIUM, theme)
            .aria_label(name)
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx));
        // Narrower than `outlined`, so that two buttons share a row of the callout in French.
        let button = button
            .px(BASE_08)
            .gap(BASE_04)
            .children(keystroke.map(|keystroke| keys(key_caps(keystroke, self.language), theme)));
        self.reveal_on_tab(id, button)
    }

    /// « Ouvrir les paramètres », on « Général », the first page.
    pub(super) fn open_settings_button(&self, id: &'static str, theme: &Theme) -> AnyElement {
        self.callout_button(
            id,
            self.text(Key::SourceOpenSettings).into(),
            None,
            Box::new(OpenGeneralSettings),
            theme,
        )
    }

    /// « Actualiser » with its F5 key cap, under a message that asks for a refresh.
    pub(super) fn refresh_button(&self, id: &'static str, theme: &Theme) -> AnyElement {
        self.callout_button(
            id,
            self.text(Key::ShellRefreshAutomationPropertiesName).into(),
            Some(keystroke::REFRESH),
            Box::new(Refresh),
            theme,
        )
    }
}

/// The filled button of a callout, the answer it expects. Not built on `super::control`, whose
/// hover fill would hide the accent.
fn filled_button(
    id: &'static str,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    // The focus colour is the accent's own in the dark theme: the text colour stands out of
    // the fill in both.
    let focused = theme.text;
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .tab_index(0)
        .rounded(radius::SM)
        .border_1()
        .border_color(theme.accent)
        .bg(theme.accent)
        .cursor_pointer()
        .focus_visible(move |style| style.border_color(focused))
        .text_color(theme.tl_on_fill)
        .font_weight(FontWeight::MEDIUM)
        .hover(|style| style.opacity(0.9))
        .active(|style| style.opacity(0.8))
        .h(size::BUTTON_MEDIUM)
        .px(BASE_08)
        .flex()
        .items_center()
        .whitespace_nowrap()
        .child(label)
}
