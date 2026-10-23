//! The « Données » page: « Exporter… », « Importer… » and the copy the last import saved, each
//! row telling how its action went. The import asks first in the window's dialog, drawn by
//! [`SettingsView::dialog`].

use gpui::{Context, FontWeight, Pixels, SharedString, Window, div, prelude::*, px};

use super::{RowText, SettingsView, group, outlined, row_frame};
use crate::settings::data::{Action, ActionRow, Button, DataPage, Row};
use crate::theme::spacing::*;
use crate::theme::{Theme, size as token};
use crate::transfer;
use crate::widgets;

/// The buttons of the rows line up, whatever their labels.
const BUTTON_MIN_WIDTH: Pixels = px(112.);

impl From<&ActionRow> for RowText {
    fn from(row: &ActionRow) -> Self {
        Self {
            title: row.title.into(),
            description: row.description.clone().into(),
            tone: row.tone,
            link: None,
            status: row.status.clone().map(SharedString::from),
            error: row.error,
        }
    }
}

impl SettingsView {
    /// Shows `page`, its dialog focused as when it opens, for the screenshots.
    pub fn show_data(&mut self, page: DataPage) {
        self.focus_dialog = page.dialog().is_some();
        self.data = page;
    }

    pub(super) fn data_page(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.data.view();
        let count = page.rows.len();
        let rows: Vec<_> = page
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.data_row(row, index + 1 == count, theme, cx))
            .collect();
        self.page_body(
            page.title,
            page.status.map(SharedString::from),
            page.busy,
            [group(("section", 0_usize), Some(page.heading), rows, theme).into_any_element()],
            theme,
        )
    }

    fn data_row(
        &self,
        row: &ActionRow,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (id, which) = (
            match row.row {
                Row::Export => "export-data",
                Row::Import => "import-data",
                Row::Backup => "show-backup",
            },
            row.row,
        );
        let button = if row.enabled {
            let button = outlined(id, row.label, token::BUTTON_MEDIUM, theme)
                .min_w(BUTTON_MIN_WIDTH)
                .justify_center();
            match row.button {
                Button::Run(action) => button.on_click(
                    cx.listener(move |view, _, window, cx| view.run_transfer(action, window, cx)),
                ),
                Button::Reveal => {
                    button.on_click(cx.listener(move |view, _, _, cx| view.reveal(which, cx)))
                }
            }
        } else {
            disabled(id, row.label, theme)
        };
        let reveal = row.reveal.map(|label| {
            div().pt(BASE_04).flex().child(
                widgets::disclosure(("reveal", which as usize), label, false, theme)
                    .on_click(cx.listener(move |view, _, _, cx| view.reveal(which, cx))),
            )
        });
        row_frame(row.into(), button.into_any_element(), reveal, last, theme).into_any_element()
    }

    /// « Exporter… » or « Importer… ».
    fn run_transfer(&mut self, action: Action, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.data.start(action) else {
            return;
        };
        let view = cx.entity().downgrade();
        match action {
            Action::Export => transfer::export(
                directory,
                move |exported, cx| {
                    view.update(cx, |view, cx| {
                        view.data.exported(exported);
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            ),
            Action::Import => transfer::choose_import(
                directory,
                move |read, cx| {
                    view.update(cx, |view, cx| {
                        view.data.read(read);
                        view.focus_dialog = view.data.dialog().is_some();
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            ),
        }
        cx.notify();
    }

    /// Shows the file of `row` selected in File Explorer.
    fn reveal(&self, row: Row, cx: &mut Context<Self>) {
        if let Some(file) = self.data.revealed(row) {
            cx.reveal_path(file);
        }
    }

    /// « Remplacer et redémarrer »: WorkTime restarts on the imported data.
    pub(super) fn confirm_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, directory)) = self.data.confirm() else {
            return;
        };
        window.focus(&self.focus_handle, cx);
        cx.notify();
        let view = cx.entity().downgrade();
        transfer::import(
            directory,
            file,
            move |cx| {
                view.update(cx, |view, cx| {
                    view.data.import_failed();
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
    }

    /// « Annuler », or Escape: this PC's data stays.
    pub(super) fn cancel_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.data.cancel();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
}

/// A button that cannot act now, outlined like the others but dimmed.
fn disabled(id: &'static str, label: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    widgets::disabled_control(id, label.into(), theme)
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
        .child(label)
}
