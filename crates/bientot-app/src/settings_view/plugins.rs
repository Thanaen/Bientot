//! The « Plugins » page: « Installer depuis un fichier… » and the plugins folder, then the
//! installed plugins, each with its switch and « Retirer ». A `.bientot-plugin` file dropped on
//! the page installs too. The consent dialog is drawn by [`SettingsView::dialog`].

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    Context, ExternalPaths, FontWeight, Pixels, Role, SharedString, Window, div, prelude::*, px,
};

use super::dialog::warning;
use super::{RowText, SettingsView, field_error, group, outlined, row_frame, switch_frame};
use crate::file_dialogs;
use crate::plugins;
use crate::settings::RowTone;
use crate::settings::plugins::{AddRow, PluginRow, PluginsPage};
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, size as token, text};
use crate::widgets;

/// The buttons of the rows line up, whatever their labels.
const BUTTON_MIN_WIDTH: Pixels = px(112.);

/// The extension of a plugin package.
const EXTENSION: &str = "bientot-plugin";
const EXTENSIONS: [&str; 1] = [EXTENSION];

impl SettingsView {
    /// Shows `page` listing `installed`, its dialog focused as when it opens, for the
    /// screenshots.
    pub fn show_plugins(&mut self, page: PluginsPage, installed: Vec<plugins::Installed>) {
        self.focus_dialog = page.dialog().is_some();
        self.plugins = page;
        self.plugins_shown = Some(Arc::new(installed));
    }

    pub(super) fn plugins_page(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let installed = self
            .plugins_shown
            .clone()
            .or_else(|| plugins::installed(cx));
        let page = self.plugins.view(installed.as_deref().map(Vec::as_slice));
        let count = page.rows.len();
        let rows: Vec<gpui::AnyElement> = if let Some(empty) = page.empty {
            vec![
                div()
                    .py(BASE_12)
                    .text_color(theme.text_muted)
                    .child(empty)
                    .into_any_element(),
            ]
        } else {
            page.rows
                .iter()
                .enumerate()
                .map(|(index, row)| self.plugin_row(index, row, index + 1 == count, theme, cx))
                .collect()
        };
        let install = page.install.enabled;
        let add = [
            self.add_row(
                "install-plugin",
                &page.install,
                false,
                theme,
                cx,
                |view, cx| view.choose_package(cx),
            ),
            self.add_row(
                "open-plugins-folder",
                &page.folder,
                true,
                theme,
                cx,
                |_, cx| plugins::open_folder(cx),
            ),
        ];
        let accent = theme.accent_background;
        self.page_body(
            page.title,
            page.status.map(SharedString::from),
            page.busy,
            [
                // The install and how it went stay in view, however many plugins follow.
                group(("section", 0_usize), Some(page.add_heading), add, theme).into_any_element(),
                group(
                    ("section", 1_usize),
                    Some(page.installed_heading),
                    rows,
                    theme,
                )
                .into_any_element(),
            ],
            theme,
        )
        .when(install, |body| {
            body.drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(accent))
                .on_drop(cx.listener(|view, paths: &ExternalPaths, _, cx| {
                    if let Some(file) = package(paths.paths()) {
                        view.read_package(file, cx);
                    }
                }))
        })
    }

    fn plugin_row(
        &self,
        index: usize,
        row: &PluginRow,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let switch = switch_frame(
            ("plugin-switch", index),
            row.switch_name.clone(),
            row.on,
            row.switchable,
            theme,
        );
        let switch = if row.switchable {
            let (id, on) = (row.id.clone(), !row.on);
            switch.on_click(cx.listener(move |_, _, _, cx| plugins::turn(&id, on, cx)))
        } else {
            switch
        };
        let (id, name) = (row.id.clone(), row.name.clone());
        let remove = outlined(
            ("remove-plugin", index),
            row.remove,
            token::BUTTON_MEDIUM,
            theme,
        )
        .aria_label(row.remove_name.clone())
        .on_click(cx.listener(move |view, _, _, cx| {
            let removed = plugins::remove(&id, cx).is_ok();
            view.plugins.removed(name.clone(), removed);
            cx.notify();
        }));
        let controls = div()
            .flex()
            .items_center()
            .gap(BASE_16)
            .child(remove)
            .child(switch);
        let details = div()
            .pt(BASE_04)
            .flex()
            .flex_col()
            .gap(BASE_04)
            .type_style(text::LABEL_SM)
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child(row.capabilities.clone()),
            )
            .children(row.problem.clone().map(|problem| {
                if problem.blocking {
                    field_error(problem.text, theme).into_any_element()
                } else {
                    // As far below the capabilities as an error, which has its own margin.
                    div()
                        .mt(BASE_04)
                        .child(warning(problem.text.into(), theme))
                        .into_any_element()
                }
            }));
        let text = RowText {
            title: row.name.clone().into(),
            description: row.description.clone().into(),
            tone: RowTone::Normal,
            link: None,
            status: None,
            error: None,
        };
        div()
            .id(("plugin", index))
            .role(Role::Group)
            .aria_label(row.name.clone())
            .child(row_frame(
                text,
                controls.into_any_element(),
                Some(details),
                last,
                theme,
            ))
            .into_any_element()
    }

    fn add_row(
        &self,
        id: &'static str,
        row: &AddRow,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        act: fn(&mut Self, &mut Context<Self>),
    ) -> gpui::AnyElement {
        let button = if row.enabled {
            outlined(id, row.label, token::BUTTON_MEDIUM, theme)
                .min_w(BUTTON_MIN_WIDTH)
                .justify_center()
                .on_click(cx.listener(move |view, _, _, cx| act(view, cx)))
        } else {
            widgets::disabled_control(id, row.label.into(), theme)
                .flex_none()
                .h(token::BUTTON_MEDIUM)
                .min_w(BUTTON_MIN_WIDTH)
                .px(BASE_12)
                .flex()
                .items_center()
                .justify_center()
                .border_color(theme.border_variant)
                .bg(theme.element)
                .font_weight(FontWeight::NORMAL)
                .whitespace_nowrap()
                .child(row.label)
        };
        let text = RowText {
            title: row.title.into(),
            description: row.description.into(),
            tone: if row.enabled {
                RowTone::Normal
            } else {
                RowTone::Unavailable
            },
            link: None,
            status: None,
            error: None,
        };
        // Under the row's line, as an error: the button stays where it is.
        let status = row.status.clone().map(|status| done(status, theme));
        let error = row.error.clone().map(|error| field_error(error, theme));
        row_frame(text, button.into_any_element(), None, last, theme)
            .children(status)
            .children(error)
            .into_any_element()
    }

    /// « Installer depuis un fichier… »: the open dialog, then the package's checks.
    fn choose_package(&mut self, cx: &mut Context<Self>) {
        if !self.plugins.start() {
            return;
        }
        cx.notify();
        let file_type = self.text(Key::SettingsPluginsFileType).to_owned();
        cx.spawn(async move |view, cx| {
            let picked = file_dialogs::pick_open(file_type, &EXTENSIONS, cx).await;
            view.update(cx, |view, cx| {
                match picked {
                    Ok(Some(file)) => view.read_checked(file, cx),
                    Ok(None) => view.plugins.dismissed(),
                    Err(_) => {
                        view.plugins.read(Err(plugins::Refused::Unreadable));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A file dropped on the page.
    fn read_package(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        if self.plugins.start() {
            self.read_checked(file, cx);
            cx.notify();
        }
    }

    fn read_checked(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        plugins::read_package(
            file,
            move |read, cx| {
                view.update(cx, |view, cx| {
                    match view.plugins.read(read) {
                        Some(offered) => view.install(offered, cx),
                        None => view.focus_dialog = view.plugins.dialog().is_some(),
                    }
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
    }

    fn install(&mut self, offered: plugins::Offered, cx: &mut Context<Self>) {
        let consent = offered.consent.clone();
        let installed = plugins::install(offered, cx).is_ok();
        self.plugins.installed(consent, installed);
    }

    /// « Installer » or « Mettre à jour » in the consent dialog.
    pub(super) fn confirm_consent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offered) = self.plugins.confirm() else {
            return;
        };
        window.focus(&self.focus_handle, cx);
        self.install(offered, cx);
        cx.notify();
    }

    /// « Annuler », or Escape: nothing is installed.
    pub(super) fn cancel_consent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plugins.cancel();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
}

/// How the install went, after a check in the success colour.
fn done(message: String, theme: &Theme) -> impl IntoElement {
    let message = SharedString::from(message);
    div()
        .id("plugin-status")
        .role(Role::Status)
        .aria_label(message.clone())
        .mt(BASE_04)
        .flex()
        .items_start()
        .gap(BASE_06)
        .type_style(text::LABEL_SM)
        .child(
            div()
                .h(text::LABEL_SM.line_height)
                .flex()
                .items_center()
                .child(widgets::icon("circle-check", theme.success).size(token::ICON_SM)),
        )
        .child(div().flex_1().min_w_0().child(message))
}

/// The first plugin package among dropped `paths`.
fn package(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plugin_package_among_the_dropped_files_installs() {
        let paths = [
            PathBuf::from(r"C:\Users\me\notes.txt"),
            PathBuf::from(r"C:\Users\me\Focus.BIENTOT-PLUGIN"),
        ];
        assert_eq!(package(&paths), Some(paths[1].clone()));
        assert_eq!(package(&paths[..1]), None);
    }
}
