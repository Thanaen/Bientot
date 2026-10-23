//! The window WorkTime opens instead of the day when it cannot start. Its texts come from
//! [`StartupFailure`]; « Exporter le diagnostic… » saves the ZIP through the tray's flow, and
//! « Fermer » or the close button quits.

use gpui::{
    App, AppContext as _, Bounds, Context, Div, FocusHandle, FontWeight, Global, KeyBinding,
    Pixels, Role, ScrollHandle, Stateful, Text, Window, WindowBounds, WindowHandle, WindowOptions,
    actions, div, prelude::*, px, size,
};

use crate::diagnostics;
use crate::flyout;
use crate::settings_view::{outlined, spinner};
use crate::startup_failure::{Action, FailurePage, StartupFailure};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, text};
use crate::week_view::tinted;
use crate::widgets;

actions!(startup_failure, [Close, FocusNext, FocusPrevious]);

const KEY_CONTEXT: &str = "StartupFailure";

pub const WINDOW_WIDTH: Pixels = px(520.);
pub const WINDOW_HEIGHT: Pixels = px(400.);

/// Wide enough for « Exporter le diagnostic… », so that neither button moves while the export
/// runs.
const EXPORT_MIN_WIDTH: Pixels = px(196.);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Close, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
    ]);
}

#[derive(Default)]
struct FailureWindow(Option<WindowHandle<StartupFailureView>>);

impl Global for FailureWindow {}

/// Opens the window of `failure`. Closing it quits WorkTime, which has nothing else to show.
pub fn open(failure: StartupFailure, cx: &mut App) -> anyhow::Result<()> {
    cx.set_global(failure.language());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(WINDOW_WIDTH, WINDOW_HEIGHT),
            cx,
        ))),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(failure.page().title.into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let window = cx.open_window(options, |window, cx| {
        window.on_window_should_close(cx, |_, cx| {
            cx.quit();
            true
        });
        cx.new(|cx| StartupFailureView::new(failure, window, cx))
    })?;
    cx.set_global(FailureWindow(Some(window)));
    Ok(())
}

/// Brings the open failure to the front, as a later launch does; whether one is open.
pub fn activate(cx: &mut App) -> bool {
    cx.default_global::<FailureWindow>()
        .0
        .is_some_and(|window| {
            window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        })
}

pub struct StartupFailureView {
    failure: StartupFailure,
    focus_handle: FocusHandle,
    detail_scroll: ScrollHandle,
}

impl StartupFailureView {
    pub fn new(failure: StartupFailure, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            failure,
            focus_handle,
            detail_scroll: ScrollHandle::new(),
        }
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let contents = self.failure.contents().clone();
        let view = cx.entity().downgrade();
        let started = diagnostics::export_with(
            move |_| contents,
            move |outcome, cx| {
                view.update(cx, |view, cx| {
                    view.failure.exported(outcome);
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
        if started {
            self.failure.start_export();
            cx.notify();
        }
    }

    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    /// The error and its causes, one row each, in the type of the values WorkTime shows. The
    /// rows take the room the rest of the page leaves, at least three of them, and scroll past
    /// it.
    fn detail(&self, page: &FailurePage, theme: &Theme) -> impl IntoElement {
        let rows = div()
            .id("startup-failure-detail")
            .track_scroll(&self.detail_scroll)
            .size_full()
            .overflow_y_scroll()
            .py(BASE_06)
            .px(BASE_12)
            .flex()
            .flex_col()
            .gap(BASE_04)
            .children(page.detail.iter().enumerate().map(|(index, line)| {
                Text::new(("startup-failure-detail", index).into(), line.clone())
            }));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .child(widgets::small(page.details_header, theme.text_muted))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(
                        text::WT_VALUE_SM.line_height * 3. + BASE_04 * 2. + BASE_06 * 2. + px(2.),
                    )
                    .rounded(radius::MD)
                    .border_1()
                    .border_color(theme.border_variant)
                    .bg(theme.surface)
                    .type_style(text::WT_VALUE_SM)
                    .text_color(theme.text_muted)
                    .child(rows)
                    .child(widgets::scroll_thumb(self.detail_scroll.clone(), theme)),
            )
    }

    fn footer(
        &self,
        page: &FailurePage,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let close = outlined("quit", page.close, token::BUTTON_LARGE, theme)
            .on_click(cx.listener(|view, _, window, cx| view.close(&Close, window, cx)));
        let export = if page.export.enabled {
            tinted("export", page.export.name.into(), theme)
                .h(token::BUTTON_LARGE)
                .min_w(EXPORT_MIN_WIDTH)
                .px(BASE_12)
                .justify_center()
                .child(page.export.label)
                .on_click(cx.listener(|view, _, _, cx| view.export(cx)))
        } else {
            exporting(&page.export, theme)
        };
        div()
            .flex_none()
            .pt(BASE_12)
            .pb(BASE_16)
            .px(BASE_24)
            .flex()
            .items_center()
            .justify_end()
            .gap(BASE_08)
            .border_t_1()
            .border_color(theme.border_variant)
            .child(close)
            .child(export)
    }
}

/// « Exporter le diagnostic… » while it runs: disabled and busy for screen readers, out of the
/// Tab order and deaf to clicks, as the flyout's « Mettre à jour » while the Store installs. The
/// same id keeps the focus of a button pressed with the keyboard.
fn exporting(action: &Action, theme: &Theme) -> Stateful<Div> {
    let focused = theme.border_focused;
    div()
        .id("export")
        .role(Role::Button)
        .aria_label(action.name)
        // GPUI has no setter for these states, only this hook on the node.
        .a11y_synthetic_children(|children| {
            let node = children.parent_node();
            node.set_disabled();
            node.set_busy();
        })
        .tab_index(0)
        .tab_stop(false)
        .focus_visible(move |style| style.border_color(focused))
        .h(token::BUTTON_LARGE)
        .min_w(EXPORT_MIN_WIDTH)
        .px(BASE_12)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(BASE_04)
        .rounded(radius::SM)
        .border_1()
        .border_color(theme.border_variant)
        .bg(theme.element)
        .text_color(theme.text_placeholder)
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(spinner(theme))
        .child(action.label)
}

impl Render for StartupFailureView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let page = self.failure.page();
        let header = div()
            .flex_none()
            .flex()
            .items_start()
            .gap(BASE_12)
            .child(
                div()
                    .h(text::HEADLINE_SM.line_height)
                    .flex()
                    .items_center()
                    .child(widgets::icon("circle-alert", theme.error)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(BASE_04)
                    .child(
                        div()
                            .id("heading")
                            .role(Role::Heading)
                            .aria_label(page.heading)
                            .type_style(text::HEADLINE_SM)
                            .child(page.heading),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(flyout::readable("startup-failure-message", page.message)),
                    ),
            );
        let notice = page.notice.as_ref().map(|notice| {
            widgets::notice_box("export-notice", notice, &theme)
                .flex_none()
                .child(div().flex_1().min_w_0().child(notice.message.clone()))
        });
        // Only the details give way when a notice comes in, so that the privacy footnote stays
        // above the footer.
        let content = div()
            .flex_1()
            .min_h_0()
            .pt(BASE_24)
            .pb(BASE_16)
            .px(BASE_24)
            .flex()
            .flex_col()
            .gap(BASE_12)
            .child(header)
            .child(self.detail(&page, &theme))
            .children(notice)
            .child(
                div()
                    .flex_none()
                    .child(widgets::footnote(page.privacy, &theme)),
            );
        div()
            .id("startup-failure")
            .role(Role::AlertDialog)
            .aria_label(page.heading)
            .aria_description(page.message)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.editor)
            .text_color(theme.text)
            .type_style(text::LABEL_MD)
            .child(widgets::title_bar(
                page.title,
                [],
                cx.listener(|_, _, _, cx| cx.quit()),
                &theme,
            ))
            .child(content)
            .child(self.footer(&page, &theme, cx))
    }
}
