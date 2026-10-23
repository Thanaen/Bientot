//! Draws the command palette over the flyout or the week, and runs the chosen command through
//! the action its shortcut dispatches. Its state lives in [`Palette`].

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    Action, App, Context, Entity, EventEmitter, FocusHandle, Focusable as _, FontWeight,
    HighlightStyle, KeyBinding, MouseButton, Pixels, Role, ScrollHandle, Stateful, StyledText,
    Subscription, Window, actions, canvas, div, prelude::*, px, relative,
};

use crate::flyout::{self, key_caps, keys};
use crate::palette::{Command, Palette, Row, Situation};
use crate::strings::{Key, Language};
use crate::text_input::TextInput;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size, text};
use crate::week_view;
use crate::widgets;

actions!(palette, [SelectPrevious, SelectNext, RunSelected, Dismiss]);

const KEY_CONTEXT: &str = "Palette";

/// The keystrokes of the palette's actions, read back for the key caps of its footer.
mod keystroke {
    pub const SELECT_PREVIOUS: &str = "up";
    pub const SELECT_NEXT: &str = "down";
    pub const RUN: &str = "enter";
    pub const DISMISS: &str = "escape";
}

/// Where the palette's top edge sits, as a share of the height it opens over.
const TOP: f32 = 0.15;
const QUERY_HEIGHT: Pixels = BASE_48;
const HEADING_HEIGHT: Pixels = BASE_24;
const ROW_HEIGHT: Pixels = size::BUTTON_MEDIUM;
const LIST_PADDING: Pixels = BASE_04;

/// The height of the list, in a palette that ends a gutter above the bottom of the `room` it
/// opens over: all of it when it fits, otherwise the rows that fit and half of the next, so
/// that it shows it scrolls.
fn list_height(room: Pixels, heading: bool, rows: usize) -> Pixels {
    let chrome = QUERY_HEIGHT + size::STATUSBAR_HEIGHT + px(2.);
    let space = room * (1. - TOP) - BASE_16 - chrome;
    let above = LIST_PADDING + if heading { HEADING_HEIGHT } else { px(0.) };
    let all = above + ROW_HEIGHT * rows.max(1) as f32 + LIST_PADDING;
    if all <= space {
        return all;
    }
    let whole = ((space - above) / ROW_HEIGHT - 0.5).floor().max(0.);
    above + ROW_HEIGHT * (whole + 0.5)
}

pub fn bind_keys(cx: &mut App) {
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new(keystroke::SELECT_PREVIOUS, SelectPrevious, context),
        KeyBinding::new(keystroke::SELECT_NEXT, SelectNext, context),
        KeyBinding::new(keystroke::RUN, RunSelected, context),
        KeyBinding::new(keystroke::DISMISS, Dismiss, context),
    ]);
}

pub enum Event {
    Run(Command),
    Dismissed,
}

pub struct PaletteView {
    state: Palette,
    language: Language,
    query: Entity<TextInput>,
    scroll: ScrollHandle,
    /// The height of the surface the palette opens over, once drawn.
    room: Rc<Cell<Option<Pixels>>>,
    _query_changed: Subscription,
}

impl EventEmitter<Event> for PaletteView {}

impl PaletteView {
    fn new(
        situation: Situation,
        room: Rc<Cell<Option<Pixels>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = cx.try_global::<Language>().copied().unwrap_or_default();
        let query = cx.new(|cx| {
            let mut input = TextInput::new(
                language.text(Key::PaletteSearchName),
                language.text(Key::PalettePlaceholder),
                None,
                cx,
            );
            input.frameless = true;
            input
        });
        window.focus(&query.focus_handle(cx), cx);
        let query_changed = cx.observe(&query, |view, query, cx| {
            view.state.set_query(query.read(cx).text());
            view.scroll_to_selected();
            cx.notify();
        });
        Self {
            state: Palette::new(situation, language),
            language,
            query,
            scroll: ScrollHandle::new(),
            room,
            _query_changed: query_changed,
        }
    }

    pub fn set_situation(&mut self, situation: Situation, cx: &mut Context<Self>) {
        self.state.set_situation(situation);
        cx.notify();
    }

    /// Types `text` in the query field, for the screenshots.
    pub fn type_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.query.update(cx, |query, cx| query.set_text(text, cx));
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.state.select_previous();
        self.scroll_to_selected();
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.state.select_next();
        self.scroll_to_selected();
        cx.notify();
    }

    fn run_selected(&mut self, _: &RunSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(command) = self.state.chosen() {
            cx.emit(Event::Run(command));
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(Event::Dismissed);
    }

    /// The heading is the list's first child while it shows.
    fn scroll_to_selected(&self) {
        let heading = usize::from(self.state.heading().is_some());
        self.scroll.scroll_to_item(self.state.selected() + heading);
    }

    fn row(
        &self,
        index: usize,
        row: Row,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = index == self.state.selected();
        let (hover, active) = (theme.element_hover, theme.element_active);
        let command = row.command;
        let label =
            StyledText::new(row.label.clone()).with_highlights(highlights(&row, selected, theme));
        div()
            .id(("palette-row", index))
            .role(Role::ListBoxOption)
            .aria_label(row.name)
            .aria_selected(selected)
            .h(size::BUTTON_MEDIUM)
            .flex_none()
            .px(BASE_08)
            .flex()
            .items_center()
            .gap(BASE_08)
            .rounded(radius::SM)
            .cursor_pointer()
            .when(selected, |row| row.bg(active))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(label),
            )
            .children(
                command
                    .keystroke()
                    .map(|keystroke| keys(key_caps(keystroke, self.language), theme)),
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::Run(command))))
    }

    /// Choosing and running need a row: with none, only « Fermer » is left.
    fn footer(&self, any_row: bool, theme: &Theme) -> impl IntoElement {
        let hint = |id: &'static str, keystrokes: &[&str], label: Key| {
            let label = self.language.text(label);
            let caps = keystrokes
                .iter()
                .flat_map(|keystroke| key_caps(keystroke, self.language))
                .collect();
            div()
                .id(id)
                .role(Role::Group)
                .aria_label(label)
                .flex()
                .items_center()
                .gap(BASE_06)
                .child(keys(caps, theme))
                .child(label)
        };
        div()
            .h(size::STATUSBAR_HEIGHT)
            .flex_none()
            .px(BASE_12)
            .flex()
            .items_center()
            .gap(BASE_16)
            .border_t_1()
            .border_color(theme.border_variant)
            .bg(theme.background)
            .type_style(text::LABEL_SM)
            .text_color(theme.text_muted)
            .whitespace_nowrap()
            .when(any_row, |footer| {
                footer
                    .child(hint(
                        "select-hint",
                        &[keystroke::SELECT_PREVIOUS, keystroke::SELECT_NEXT],
                        Key::PaletteSelectHint,
                    ))
                    .child(hint("run-hint", &[keystroke::RUN], Key::PaletteRunHint))
            })
            .child(hint(
                "close-hint",
                &[keystroke::DISMISS],
                Key::PaletteCloseHint,
            ))
    }
}

/// The domain muted and the part the query matched in the accent, both stronger on the
/// selected row's fill.
fn highlights(row: &Row, selected: bool, theme: &Theme) -> Vec<(Range<usize>, HighlightStyle)> {
    let (accent, muted) = if selected {
        (theme.accent_on_active, theme.text_muted_on_active)
    } else {
        (theme.accent, theme.text_muted)
    };
    let mut cuts = vec![0, row.verb_start, row.label.len()];
    if let Some(hit) = &row.hit {
        cuts.extend([hit.start, hit.end]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .map(|cut| {
            let range = cut[0]..cut[1];
            let hit = row
                .hit
                .as_ref()
                .is_some_and(|hit| hit.start <= range.start && range.end <= hit.end);
            let color = if hit {
                accent
            } else if range.end <= row.verb_start {
                muted
            } else {
                theme.text
            };
            let style = HighlightStyle {
                color: Some(color),
                font_weight: hit.then_some(FontWeight::MEDIUM),
                ..HighlightStyle::default()
            };
            (range, style)
        })
        .collect()
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let commands = self.language.text(Key::PaletteCommands);
        let rows = self.state.rows();
        let room = self.room.get();
        let list_height =
            room.map(|room| list_height(room, self.state.heading().is_some(), rows.len()));
        let rows: Vec<_> = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| self.row(index, row, &theme, cx).into_any_element())
            .collect();
        let any_row = !rows.is_empty();
        let empty = (!any_row).then(|| {
            let message = self.language.text(Key::PaletteNoMatch);
            div()
                .id("no-match")
                .role(Role::Status)
                .aria_label(message)
                .h(size::BUTTON_MEDIUM)
                .px(BASE_08)
                .flex()
                .items_center()
                .text_color(theme.text_muted)
                .child(message)
        });
        let heading = self.state.heading().map(|heading| {
            div()
                .h(HEADING_HEIGHT)
                .flex_none()
                .px(BASE_08)
                .pb(BASE_02)
                .flex()
                .items_end()
                .type_style(text::LABEL_SM)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(heading)
        });
        div()
            .id("palette")
            .role(Role::Dialog)
            .aria_label(self.language.text(Key::PaletteDialogName))
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::run_selected))
            .on_action(cx.listener(Self::dismiss))
            .occlude()
            .w(size::PALETTE_WIDTH)
            .max_w_full()
            .flex()
            .flex_col()
            .bg(theme.elevated_surface)
            .border_1()
            .border_color(theme.border)
            .rounded(radius::LG)
            .shadow(theme.shadow_palette())
            .overflow_hidden()
            // Until the surface below is measured, as it is on the first frame.
            .when(room.is_none(), |palette| palette.opacity(0.))
            .type_style(text::LABEL_MD)
            .text_color(theme.text)
            .child(
                div()
                    .h(QUERY_HEIGHT)
                    .flex_none()
                    .px(BASE_12)
                    .flex()
                    .items_center()
                    .gap(BASE_08)
                    .border_b_1()
                    .border_color(theme.border_variant)
                    .child(widgets::icon("search", theme.text_muted))
                    .child(div().flex_1().min_w_0().child(self.query.clone())),
            )
            .child(
                div()
                    .id("commands")
                    .role(Role::ListBox)
                    .aria_label(commands)
                    .when_some(list_height, |list, height| list.h(height))
                    .flex_none()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(LIST_PADDING)
                    .flex()
                    .flex_col()
                    .children(heading)
                    .children(rows)
                    .children(empty),
            )
            .child(self.footer(any_row, &theme))
    }
}

/// The palette open over a window, until it runs a command or closes.
pub struct Opened {
    view: Entity<PaletteView>,
    room: Rc<Cell<Option<Pixels>>>,
    _events: Subscription,
}

impl Opened {
    /// Opens the palette over the window of `V`, focused. Once it runs a command or closes,
    /// `slot` lets it go and `host` takes the focus back, so the command's action reaches the
    /// window as its shortcut would.
    pub fn new<V: 'static>(
        situation: Situation,
        host: FocusHandle,
        slot: fn(&mut V) -> &mut Option<Opened>,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let room = Rc::default();
        let view = cx.new(|cx| PaletteView::new(situation, Rc::clone(&room), window, cx));
        let events = cx.subscribe_in(&view, window, move |owner, _, event, window, cx| {
            slot(owner).take();
            window.focus(&host, cx);
            if let Event::Run(command) = event {
                window.dispatch_action(action(*command), cx);
            }
            cx.notify();
        });
        Self {
            view,
            room,
            _events: events,
        }
    }

    pub fn view(&self) -> &Entity<PaletteView> {
        &self.view
    }

    pub fn set_situation(&self, situation: Situation, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.set_situation(situation, cx));
    }

    /// The scrim over the window's content and the palette on it; the caller places it. The
    /// scrim measures the room it covers, and the palette fits its list in it.
    pub fn overlay(&self, theme: &Theme) -> Stateful<gpui::Div> {
        let view = self.view.downgrade();
        let room = Rc::clone(&self.room);
        let measured = self.view.downgrade();
        let measure = canvas(
            move |bounds, window, _| {
                if room.get() != Some(bounds.size.height) {
                    room.set(Some(bounds.size.height));
                    window.on_next_frame(move |_, cx| {
                        measured.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .id("palette-scrim")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(theme.scrim)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                view.update(cx, |_, cx| cx.emit(Event::Dismissed)).ok();
            })
            .child(measure)
            .child(
                div()
                    .absolute()
                    .top(relative(TOP))
                    .left_0()
                    .right_0()
                    .px(BASE_16)
                    .flex()
                    .justify_center()
                    .child(self.view.clone()),
            )
    }
}

/// The action the command's shortcut dispatches.
fn action(command: Command) -> Box<dyn Action> {
    match command {
        Command::TogglePause => Box::new(flyout::TogglePause),
        Command::AdjustTime => Box::new(flyout::AdjustTime),
        Command::PreviousDay => Box::new(week_view::PreviousDay),
        Command::NextDay => Box::new(week_view::NextDay),
        Command::SettleNext => Box::new(week_view::SettleNext),
        Command::Undo => Box::new(week_view::Undo),
        Command::FinishDay => Box::new(flyout::FinishDay),
        Command::OpenWeek => Box::new(flyout::OpenWeek),
        Command::OpenSettings => Box::new(flyout::OpenSettings),
        Command::OpenSettingsJson => Box::new(flyout::OpenSettingsJson),
        Command::Refresh => Box::new(flyout::Refresh),
        Command::ExportDiagnostics => Box::new(flyout::ExportDiagnostics),
        Command::Quit => Box::new(flyout::Quit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_that_fits_shows_whole_and_one_that_does_not_shows_half_a_row_more() {
        // The heading and nine rows: 4 + 24 + 9 × 28 + 4 = 284 px.
        assert_eq!(list_height(px(800.), true, 9), px(284.));
        // Over 500 px, 500 × 0.85 − 16 − 48 − 28 − 2 = 331 px fit the whole list, cut to 10.5 rows.
        assert_eq!(list_height(px(500.), true, 13), px(4. + 24. + 28. * 10.5));
        // Over 400 px, 246 px: 7.5 rows.
        assert_eq!(list_height(px(400.), true, 9), px(4. + 24. + 28. * 7.5));
        // No match still keeps the room of one row for the message.
        assert_eq!(list_height(px(400.), false, 0), px(36.));
    }
}
