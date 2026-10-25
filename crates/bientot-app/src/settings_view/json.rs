//! The « settings.json » page: the file as it is on disk, read when the page opens and never
//! written. Edits made in an editor apply at the next start.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Context, FocusHandle, FontWeight, HighlightStyle, KeyDownEvent, Pixels, Role,
    ScrollWheelEvent, StyledText, Window, div, font, prelude::*, px,
};

use super::{NAV_WIDTH, SettingsView, outlined};
use crate::assets::MONO_FONT_FAMILY;
use crate::settings::json::{Token, first_row, lines, rows, tokens_within};
use crate::strings::{Key, Language};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, text};

const LINE: Pixels = px(20.);
const NUMBER_WIDTH: Pixels = px(40.);
/// What the window holds besides the lines: the title bar, the header, the box's padding and
/// border, the note under it and the bottom margin.
const CHROME: Pixels = px(32. + (20. + 28. + 16.) + (2. * 8. + 2.) + (8. + 16.) + 24.);

pub(super) struct JsonPage {
    file: Option<PathBuf>,
    /// The file's folder, as the header names it.
    pub(super) folder: Option<String>,
    /// The file's lines while the page shows.
    lines: Option<Vec<String>>,
    /// The first row shown. Whole rows scroll, so that none is ever cut.
    top: isize,
    /// Wheel movement short of a whole row.
    pending: Pixels,
    /// The rows the box showed last.
    rows: usize,
    focus: FocusHandle,
}

impl JsonPage {
    pub(super) fn new(file: Option<PathBuf>, focus: FocusHandle) -> Self {
        Self {
            folder: file
                .as_deref()
                .and_then(Path::parent)
                .map(|folder| folder.display().to_string()),
            file,
            lines: None,
            top: 0,
            pending: px(0.),
            rows: 1,
            focus,
        }
    }

    /// Reads the file when the page opens, and forgets it when it closes.
    pub(super) fn show(&mut self, shown: bool) {
        if !shown {
            self.lines = None;
            self.top = 0;
        } else if self.lines.is_none() {
            self.lines = Some(self.file.as_deref().map(read).unwrap_or_default());
        }
    }

    pub(super) fn scroll_to_end(&mut self) {
        self.top = isize::MAX / 2;
    }

    fn scroll(&mut self, lines: isize, cx: &mut Context<SettingsView>) {
        self.top = self.top.saturating_add(lines);
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<SettingsView>) {
        let rows = self.rows as isize;
        match event.keystroke.key.as_str() {
            "up" => self.scroll(-1, cx),
            "down" => self.scroll(1, cx),
            "pageup" => self.scroll(-rows, cx),
            "pagedown" => self.scroll(rows, cx),
            "home" => self.scroll(isize::MIN / 2, cx),
            "end" => self.scroll(isize::MAX / 2, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<SettingsView>) {
        self.pending -= event.delta.pixel_delta(LINE).y;
        let lines = (self.pending / LINE).trunc();
        if lines != 0. {
            self.pending -= LINE * lines;
            self.scroll(lines as isize, cx);
        }
    }

    pub(super) fn render(
        &mut self,
        language: Language,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> AnyElement {
        let lines = self.lines.as_deref().unwrap_or_default();
        self.rows = ((window.viewport_size().height - CHROME) / LINE)
            .floor()
            .max(1.) as usize;
        let rows = rows(lines, columns(window));
        let first = first_row(self.top, self.rows, rows.len());
        self.top = first as isize;
        let shown = rows.into_iter().skip(first).take(self.rows).map(|row| {
            div()
                .h(LINE)
                .flex()
                .child(
                    div()
                        .flex_none()
                        .w(NUMBER_WIDTH)
                        .pr(BASE_12)
                        .flex()
                        .justify_end()
                        .text_color(theme.text_placeholder)
                        .when(row.starts_line(), |number| {
                            number.child((row.line + 1).to_string())
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pr(BASE_12)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(segment(&lines[row.line], row.range, row.indent, theme)),
                )
        });
        let title = language.text(Key::SettingsJsonNavContent);
        // The row aligns bottoms: the margin lifts the path's baseline to the title's.
        let folder = self.folder.as_ref().map(|folder| {
            div()
                .min_w_0()
                .mb(px(4.5))
                .type_style(text::LABEL_SM)
                .text_color(theme.text_muted)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis_middle()
                .child(format!("· {folder}"))
        });
        let badge = div()
            .flex_none()
            .self_center()
            .h(px(18.))
            .px(BASE_06)
            .flex()
            .items_center()
            .rounded(radius::MD)
            .border_1()
            .border_color(theme.border_variant)
            .bg(theme.element)
            .type_style(text::LABEL_SM)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .child(language.text(Key::SettingsJsonReadOnly));
        let open = self.file.clone().map(|file| {
            outlined(
                "open-file",
                language.text(Key::SettingsJsonOpenFile),
                token::BUTTON_LARGE,
                theme,
            )
            .on_click(move |_, _, cx| open(&file, cx))
        });
        let header = div()
            .flex_none()
            .pt(BASE_20)
            .px(BASE_24)
            .pb(BASE_16)
            .flex()
            .items_center()
            .gap(BASE_12)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_end()
                    .gap(BASE_08)
                    .child(
                        div()
                            .id("json-title")
                            .role(Role::Heading)
                            .aria_label(title)
                            .flex_none()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(px(16.))
                            .line_height(px(28.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .children(folder)
                    .child(badge),
            )
            .children(open);
        let focused = theme.border_focused;
        let code = div()
            .id("json-lines")
            .role(Role::Document)
            .aria_label(title)
            .track_focus(&self.focus)
            .flex_none()
            .mx(BASE_24)
            .h(LINE * self.rows as f32 + px(2. * 8. + 2.))
            .py(BASE_08)
            .overflow_hidden()
            .rounded(radius::LG)
            .border_1()
            .border_color(theme.border_variant)
            .focus_visible(move |style| style.border_color(focused))
            .bg(theme.editor)
            .type_style(text::WT_VALUE)
            .on_scroll_wheel(cx.listener(|view, event, _, cx| view.json.wheel(event, cx)))
            .on_key_down(cx.listener(|view, event, _, cx| view.json.key(event, cx)))
            .children(shown);
        div()
            .id("page")
            .role(Role::TabPanel)
            .aria_label(title)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(header)
            .child(code)
            .child(
                div()
                    .mt(BASE_08)
                    .mx(BASE_24)
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .child(language.text(Key::SettingsJsonRestartNote)),
            )
            .into_any_element()
    }
}

/// Rust opens a file sharing reading, writing and deleting on Windows, so this read never
/// blocks a save, which renames a new file over this one.
fn read(path: &Path) -> Vec<String> {
    match std::fs::read(path) {
        Ok(bytes) => lines(&String::from_utf8_lossy(&bytes)),
        Err(error) => {
            log::warn!("cannot read {}: {error}", path.display());
            Vec::new()
        }
    }
}

/// Opens the file in the editor Windows associates with `.json`.
fn open(file: &Path, cx: &App) {
    if cfg!(windows) {
        cx.open_with_system(file);
    }
}

/// The characters a row holds: the width of its text over that of a character.
fn columns(window: &Window) -> usize {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font(MONO_FONT_FAMILY));
    let Ok(advance) = text_system.advance(font_id, text::WT_VALUE.size, '0') else {
        return 1;
    };
    // Beside the text of a row: the navigation, the box's margins and border, the numbers and
    // the row's right padding.
    let beside = NAV_WIDTH + px(2. * 24. + 2.) + NUMBER_WIDTH + BASE_12;
    ((window.viewport_size().width - beside) / advance.width)
        .floor()
        .max(1.) as usize
}

/// One line of JSON, coloured as an editor colours it.
pub(super) fn line(line: &str, theme: &Theme) -> StyledText {
    segment(line, 0..line.len(), 0, theme)
}

/// The part `range` of a line of JSON after `indent` blank columns, coloured as in the whole
/// line.
fn segment(line: &str, range: Range<usize>, indent: usize, theme: &Theme) -> StyledText {
    let text = " ".repeat(indent) + &line[range.clone()];
    let mut start = indent;
    let highlights: Vec<_> = tokens_within(line, range)
        .into_iter()
        .filter_map(|(token, part)| {
            let range = start..start + part.len();
            start = range.end;
            let color = match token {
                Token::Plain => return None,
                Token::Key => theme.accent,
                Token::String => theme.success,
                Token::Literal => theme.warning,
            };
            Some((
                range,
                HighlightStyle {
                    color: Some(color),
                    ..HighlightStyle::default()
                },
            ))
        })
        .collect();
    StyledText::new(text).with_highlights(highlights)
}
