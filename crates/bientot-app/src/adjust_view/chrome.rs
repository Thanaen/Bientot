//! The title bar, the callout and the buttons of « Corriger le temps… ».

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, Hsla, Role, Transformation,
    WindowControlArea, div, percentage, prelude::*, px, rgb, transparent_black,
};

use super::{AdjustView, CANCEL_CONTEXT, Cancel, Save, keystroke};
use crate::adjust::{AdjustPage, CalloutTone};
use crate::flyout;
use crate::strings::{Key, Language};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, text};
use crate::widgets::{self, Hue};

/// The primary button keeps its width while the spinner replaces its label.
const PRIMARY_MIN_WIDTH: f32 = 148.;

impl From<CalloutTone> for Hue {
    fn from(tone: CalloutTone) -> Self {
        match tone {
            CalloutTone::Error => Hue::Error,
            CalloutTone::Warning => Hue::Warning,
        }
    }
}

/// The key caps of a button: outlined in `on_accent` on the accent fill, else the design
/// system's muted ones, which a disabled button keeps.
/// How a button's key caps look: plain beside an outlined button, outlined in the fill colour
/// of the accent button, or as dim as the label of a disabled button.
#[derive(Clone, Copy)]
enum Caps {
    Plain,
    OnAccent(Hsla),
    Disabled,
}

fn caps(keystroke: &str, look: Caps, language: Language, theme: &Theme) -> AnyElement {
    let (border, background, color) = match look {
        Caps::Plain => {
            return flyout::keys(flyout::key_caps(keystroke, language), theme).into_any_element();
        }
        Caps::OnAccent(color) => (color.opacity(0.8), transparent_black(), color.opacity(0.8)),
        Caps::Disabled => (theme.element, theme.element, theme.text_placeholder),
    };
    div()
        .flex()
        .flex_none()
        .gap(BASE_02)
        .children(
            flyout::key_caps(keystroke, language)
                .into_iter()
                .map(|cap| {
                    div()
                        .min_w(token::KEY_CAP)
                        .px(BASE_04)
                        .rounded(radius::XS)
                        .border_1()
                        .border_color(border)
                        .bg(background)
                        .type_style(text::LABEL_SM)
                        .text_color(color)
                        .flex()
                        .justify_center()
                        .child(cap)
                }),
        )
        .into_any_element()
}

impl AdjustView {
    pub(super) fn title_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = theme.element_hover;
        div()
            .h(token::TITLEBAR_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .bg(theme.background)
            .border_b_1()
            .border_color(theme.border_variant)
            .child(
                div()
                    .id("title")
                    .window_control_area(WindowControlArea::Drag)
                    .flex_1()
                    .h_full()
                    .pl(BASE_08)
                    .flex()
                    .items_center()
                    .gap(BASE_06)
                    .child(
                        // The app's mark, as in the mockup: white on its own green.
                        div()
                            .size(token::ICON_SM)
                            .rounded_full()
                            .bg(rgb(0x15803d))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(widgets::icon("check", gpui::white()).size(px(10.))),
                    )
                    .child(widgets::small(
                        self.text(Key::ShellWindowTitle),
                        theme.text_muted,
                    )),
            )
            .child(
                div()
                    .id("close")
                    .window_control_area(WindowControlArea::Close)
                    .w(token::WINDOW_CONTROL_WIDTH)
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |style| style.bg(hover))
                    .child(widgets::icon("x", theme.text_muted).size(token::ICON_XS))
                    .on_click(cx.listener(|view, _, window, cx| view.cancel(&Cancel, window, cx))),
            )
    }

    pub(super) fn callout(&self, page: &AdjustPage, theme: &Theme) -> Option<impl IntoElement> {
        let callout = page.callout.clone()?;
        let (role, name) = match callout.tone {
            CalloutTone::Error => (
                Role::Alert,
                self.text(Key::AdjustTimeErrorAutomationPropertiesName),
            ),
            CalloutTone::Warning => (Role::Status, callout.message),
        };
        Some(
            div().flex_none().px(BASE_24).pb(BASE_12).child(
                widgets::callout_box(callout.tone.into(), theme)
                    .id("callout")
                    .role(role)
                    .aria_label(name)
                    .aria_description(callout.message)
                    .child(div().flex_1().min_w_0().child(callout.message)),
            ),
        )
    }

    pub(super) fn footer(
        &self,
        page: &AdjustPage,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = theme.border_focused;
        let secondary = {
            let button = div()
                .id("cancel")
                .role(Role::Button)
                .h(token::BUTTON_LARGE)
                .px(BASE_12)
                .flex()
                .items_center()
                .gap(BASE_08)
                .rounded(radius::SM)
                .border_1()
                .border_color(theme.border)
                .whitespace_nowrap()
                .child(page.secondary)
                .child(caps(
                    keystroke::CANCEL,
                    if page.secondary_enabled {
                        Caps::Plain
                    } else {
                        Caps::Disabled
                    },
                    self.language,
                    theme,
                ));
            if page.secondary_enabled {
                let (hover, active) = (theme.element_hover, theme.element_active);
                button
                    .aria_label(flyout::shortcut_name(
                        page.secondary,
                        keystroke::CANCEL,
                        self.language,
                    ))
                    .key_context(CANCEL_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(active))
                    .focus_visible(move |style| style.border_color(focused))
                    .on_click(cx.listener(|view, _, window, cx| view.cancel(&Cancel, window, cx)))
            } else {
                button
                    .aria_label(page.secondary)
                    .text_color(theme.text_placeholder)
            }
        };
        let enabled = page.primary_enabled && !page.busy;
        let on_accent = theme.tl_on_fill;
        let (background, border, color) = if enabled {
            (theme.accent, theme.accent, on_accent)
        } else {
            (theme.element, theme.border_variant, theme.text_placeholder)
        };
        let spinner = widgets::icon("loader-circle", theme.text_muted)
            .size(token::ICON_SM)
            .with_animation(
                "saving",
                Animation::new(Duration::from_millis(900)).repeat(),
                |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
            );
        let primary = div()
            .id("save")
            .role(Role::Button)
            .aria_label(flyout::shortcut_name(
                page.primary,
                keystroke::SAVE,
                self.language,
            ))
            .h(token::BUTTON_LARGE)
            .min_w(px(PRIMARY_MIN_WIDTH))
            .px(BASE_12)
            .flex()
            .items_center()
            .justify_center()
            .gap(BASE_08)
            .rounded(radius::SM)
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(color)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .child(
                // The hidden label keeps the button's width under the spinner.
                div()
                    .relative()
                    .child(
                        div()
                            .when(page.busy, |label| label.text_color(transparent_black()))
                            .child(page.primary),
                    )
                    .children(page.busy.then(|| {
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(spinner)
                    })),
            )
            .child(caps(
                keystroke::SAVE,
                if enabled {
                    Caps::OnAccent(on_accent)
                } else {
                    Caps::Disabled
                },
                self.language,
                theme,
            ));
        let primary = if enabled {
            primary
                .tab_index(0)
                .cursor_pointer()
                .hover(move |style| style.border_color(focused))
                .focus_visible(move |style| style.border_color(focused))
                .on_click(cx.listener(|view, _, window, cx| view.save(&Save, window, cx)))
        } else {
            primary
        };
        div()
            .flex_none()
            .py(BASE_12)
            .px(BASE_16)
            .flex()
            .items_center()
            .justify_end()
            .gap(BASE_08)
            .border_t_1()
            .border_color(theme.border_variant)
            .bg(theme.surface)
            .child(secondary)
            .child(primary)
    }
}
