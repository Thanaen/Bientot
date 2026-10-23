//! The dialog over the settings window: unsaved changes, or an import to confirm. The button
//! that changes nothing is focused as it shows, and Tab moves between its two buttons.

use gpui::{Context, Pixels, Role, SharedString, div, prelude::*, px};

use super::{SettingsView, filled, outlined};
use crate::settings::DialogView;
use crate::settings::data::ImportDialog;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size as token, text};
use crate::widgets;

const DIALOG_WIDTH: Pixels = px(440.);

impl SettingsView {
    pub(super) fn dialog(
        &self,
        dialog: Dialog,
        time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let frame = dialog_frame(time);
        let focused = theme.border_focused;
        let text = dialog.text();
        let accent = |id: &'static str, label: &'static str| {
            filled(id, label, token::BUTTON_LARGE, theme)
                .focus(move |style| style.border_color(focused))
        };
        // The button that changes nothing is focused as the dialog shows.
        let buttons = match dialog {
            Dialog::Unsaved(dialog) => [
                outlined("discard", dialog.discard, token::BUTTON_LARGE, theme)
                    .focus(move |style| style.border_color(focused))
                    .track_focus(&self.action_focus)
                    .on_click(cx.listener(|view, _, window, cx| view.discard(window, cx))),
                accent("keep-editing", dialog.keep_editing)
                    .track_focus(&self.safe_focus)
                    .on_click(cx.listener(|view, _, window, cx| view.keep_editing(window, cx))),
            ],
            Dialog::Import(dialog) => [
                // Focused from the start, its ring shows what Enter does.
                outlined("cancel-import", dialog.cancel, token::BUTTON_LARGE, theme)
                    .focus(move |style| style.border_color(focused))
                    .track_focus(&self.safe_focus)
                    .on_click(cx.listener(|view, _, window, cx| view.cancel_import(window, cx))),
                accent("confirm-import", dialog.confirm)
                    .track_focus(&self.action_focus)
                    .on_click(cx.listener(|view, _, window, cx| view.confirm_import(window, cx))),
            ],
        };
        div()
            .id("scrim")
            .absolute()
            .top(token::TITLEBAR_HEIGHT)
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(theme.scrim.opacity(frame.scrim))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id(text.id)
                    .role(Role::AlertDialog)
                    .aria_label(text.title)
                    .aria_description(text.description())
                    .map(|card| frame.card.apply(card))
                    .w(DIALOG_WIDTH)
                    .rounded(radius::LG)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.editor)
                    .shadow(theme.shadow_modal())
                    .overflow_hidden()
                    .child(
                        div()
                            .py(BASE_20)
                            .px(BASE_24)
                            .flex()
                            .flex_col()
                            .gap(BASE_08)
                            .child(
                                div()
                                    .id("dialog-title")
                                    .role(Role::Heading)
                                    .type_style(text::HEADLINE_SM)
                                    .child(text.title),
                            )
                            .children(
                                text.paragraphs
                                    .into_iter()
                                    .map(|paragraph| div().child(paragraph)),
                            )
                            .children(text.warning.map(|warning| {
                                div()
                                    .flex()
                                    .items_start()
                                    .gap(BASE_08)
                                    .text_color(theme.warning)
                                    .child(
                                        div()
                                            .h(text::LABEL_MD.line_height)
                                            .flex()
                                            .items_center()
                                            .child(
                                                widgets::icon("triangle-alert", theme.warning)
                                                    .size(token::ICON_SM),
                                            ),
                                    )
                                    .child(div().flex_1().min_w_0().child(warning))
                            }))
                            .child(text.message),
                    )
                    .child(
                        div()
                            .py(BASE_12)
                            .px(BASE_24)
                            .flex()
                            .justify_end()
                            .gap(BASE_08)
                            .rounded_b(radius::LG - px(1.))
                            .border_t_1()
                            .border_color(theme.border_variant)
                            .bg(theme.surface)
                            .children(buttons),
                    ),
            )
    }
}

/// The dialog over the window: unsaved changes, or an import to confirm.
pub(super) enum Dialog {
    Unsaved(DialogView),
    Import(ImportDialog),
}

/// What a dialog says above its buttons.
struct DialogText {
    id: &'static str,
    title: &'static str,
    paragraphs: Vec<SharedString>,
    /// What the action loses, in the warning colour after the paragraphs that describe it.
    warning: Option<SharedString>,
    /// The last paragraph.
    message: &'static str,
}

impl DialogText {
    /// All it says, for the screen reader.
    fn description(&self) -> String {
        let mut parts: Vec<&str> = self.paragraphs.iter().map(SharedString::as_ref).collect();
        parts.extend(self.warning.as_deref());
        parts.push(self.message);
        parts.join(" ")
    }
}

impl Dialog {
    fn text(&self) -> DialogText {
        match self {
            Self::Unsaved(dialog) => DialogText {
                id: "unsaved",
                title: dialog.title,
                paragraphs: Vec::new(),
                warning: None,
                message: dialog.message,
            },
            Self::Import(dialog) => DialogText {
                id: "import",
                title: dialog.title,
                paragraphs: dialog.paragraphs.iter().cloned().map(Into::into).collect(),
                warning: dialog.loss.clone().map(Into::into),
                message: dialog.message,
            },
        }
    }
}

/// How far below its rest the card starts.
const RISE: Pixels = px(6.);

/// How far the unsaved-changes dialog has come in at `time`, a share of its entrance: the scrim
/// fades in on the strong ease-out and the card comes in over the page.
#[derive(Debug, PartialEq)]
struct DialogFrame {
    scrim: f32,
    card: motion::Entering,
}

fn dialog_frame(time: f32) -> DialogFrame {
    DialogFrame {
        scrim: motion::ease_out()(time),
        card: motion::slide_over(time, RISE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_comes_in_from_below_and_hidden() {
        assert_eq!(
            dialog_frame(0.),
            DialogFrame {
                scrim: 0.,
                card: motion::Entering {
                    opacity: 0.,
                    offset: RISE,
                },
            }
        );
    }

    #[test]
    fn the_dialog_card_hides_the_page_from_its_first_frames() {
        assert_eq!(dialog_frame(motion::OPAQUE_AT).card.opacity, 1.);
    }

    #[test]
    fn the_dialog_ends_at_rest() {
        assert_eq!(
            dialog_frame(1.),
            DialogFrame {
                scrim: 1.,
                card: motion::Entering {
                    opacity: 1.,
                    offset: px(0.),
                },
            }
        );
    }
}
