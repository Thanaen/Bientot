//! The language select of « Général »: a button and, while open, the list of languages under it.

use gpui::{AnyElement, Context, Role, anchored, deferred, div, prelude::*, px};

use super::{SELECT_WIDTH, SettingsView};
use crate::settings::LanguageChoice;
use crate::theme::spacing::*;
use crate::theme::{Theme, motion, radius, size as token};
use crate::widgets;

/// Where the open list stands at `time`, a share of its entrance: it comes down 4 px from its
/// button and is opaque early, since it covers the page.
fn list_pose(time: f32) -> motion::Entering {
    motion::slide_over(time, px(-4.))
}

impl SettingsView {
    /// The language select, its list `time` into its entrance while open.
    pub(super) fn language_select(
        &self,
        name: &'static str,
        value: LanguageChoice,
        time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (hover, focused) = (theme.element_hover, theme.border_focused);
        let button = div()
            .id("language")
            .role(Role::ComboBox)
            .aria_label(name)
            .aria_value(value.label(self.language))
            .aria_expanded(self.language_open)
            .w(SELECT_WIDTH)
            .h(token::BUTTON_MEDIUM)
            .px(BASE_08)
            .flex()
            .items_center()
            .justify_between()
            .rounded(radius::SM)
            .border_1()
            .border_color(theme.border)
            .bg(theme.editor)
            .tab_index(0)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .focus_visible(move |style| style.border_color(focused))
            .child(value.label(self.language))
            .child(widgets::icon("chevron-down", theme.text_muted).size(token::ICON_SM))
            .on_click(cx.listener(|view, _, _, cx| view.toggle_language_list(cx)));
        let menu = self.language_open.then(|| {
            let options = LanguageChoice::ALL
                .into_iter()
                .enumerate()
                .map(|(position, choice)| {
                    let selected = choice == value;
                    let label = choice.label(self.language);
                    div()
                        .id(("language-option", position))
                        .role(Role::ListBoxOption)
                        .aria_label(label)
                        .aria_selected(selected)
                        .h(token::BUTTON_MEDIUM)
                        .px(BASE_08)
                        .flex()
                        .items_center()
                        .gap(BASE_08)
                        .rounded(radius::SM)
                        .tab_index(0)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .when(selected, |option| option.bg(theme.element_active))
                        .child(div().flex_1().child(label))
                        .when(selected, |option| {
                            option.child(widgets::icon("check", theme.accent).size(token::ICON_SM))
                        })
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.choose_language(choice, window, cx)
                        }))
                });
            deferred(
                anchored().snap_to_window_with_margin(BASE_08).child(
                    div()
                        .id("language-options")
                        .role(Role::ListBox)
                        .aria_label(name)
                        .occlude()
                        .map(|list| list_pose(time).apply(list))
                        .mt(BASE_04)
                        .w(SELECT_WIDTH)
                        .p(BASE_04)
                        .flex()
                        .flex_col()
                        .gap(BASE_02)
                        .rounded(radius::MD)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.elevated_surface)
                        .shadow(theme.shadow_elevated())
                        .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                            view.language_open = false;
                            cx.notify();
                        }))
                        .children(options),
                ),
            )
            .with_priority(1)
        });
        div()
            .flex_none()
            .child(button)
            .children(menu)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_comes_down_from_its_button_and_hides_the_page_from_its_first_frames() {
        let start = list_pose(0.);
        assert_eq!((start.opacity, start.offset), (0., px(-4.)));
        assert_eq!(list_pose(motion::OPAQUE_AT).opacity, 1.);
        let end = list_pose(1.);
        assert_eq!((end.opacity, end.offset), (1., px(0.)));
    }
}
