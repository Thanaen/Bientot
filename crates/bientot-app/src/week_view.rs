//! The week window opened by « Ouvrir la semaine »: one timeline per weekday drawn with
//! `canvas()`, the open day's « À régler » list and the correction bubble. Its state lives in
//! [`WeekState`]; this module only draws it and turns keys and clicks into its methods.

use std::cell::Cell;
use std::rc::Rc;

use bientot_domain::accounting::TrackingState;
use bientot_domain::intervals::UtcInterval;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global, Hsla,
    KeyBinding, MouseButton, Pixels, Role, ScrollHandle, SharedString, Subscription, Window,
    WindowBounds, WindowControlArea, WindowHandle, WindowOptions, actions, div, prelude::*,
    transparent_black,
};

use crate::adjust::Closed;
use crate::flyout::{self, AdjustTime, OpenCommands, Refresh};
use crate::host::{self, Today};
use crate::live::Tracking;
use crate::palette::{Host, Situation};
use crate::palette_view::{Opened, PaletteView};
use crate::settings_view;
use crate::strings::{Key, Language};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size, tabular_figures, text};
use crate::week::{self, Answer, DayTone, Edge, LegendMark, Severity, WeekState, WeekTimeline};
use crate::widgets::{self, Hue, HueColors};
use days::bracket_glyph;
pub(crate) use days::{segment_quad, swatch};
use sizing::{Layout, WindowSizes};

mod adjust;
mod bubble;
mod days;
mod details;
mod sizing;

actions!(
    week,
    [
        PreviousDay,
        NextDay,
        SettleNext,
        Undo,
        CloseBubble,
        PreviousSegment,
        NextSegment,
        ExtendStart,
        ExtendEnd,
        StartEarlier,
        StartLater,
        EndEarlier,
        EndLater,
        FocusNext,
        FocusPrevious
    ]
);

const KEY_CONTEXT: &str = "Week";
/// « De » and « À », whose arrows and letters are their own.
pub(crate) const FIELD_CONTEXT: &str = "RangeField";
/// The week's keys that a field of the bubble keeps for itself.
const OUTSIDE_FIELDS: &str = "Week && !RangeField";

/// The keystrokes of the week's actions, read back for their key caps.
pub(crate) mod keystroke {
    pub const SETTLE_NEXT: &str = "n";
    pub const UNDO: &str = "ctrl-z";
    pub const REFRESH: &str = "f5";
    pub const ACCEPT: &str = "enter";
    pub const CLOSE: &str = "escape";
    pub const PREVIOUS_DAY: &str = "up";
    pub const NEXT_DAY: &str = "down";
    pub const ADJUST_TIME: &str = "ctrl-e";
    pub const OPEN_COMMANDS: &str = "ctrl-shift-p";
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(bindings());
}

fn bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    let outside_fields = Some(OUTSIDE_FIELDS);
    vec![
        KeyBinding::new(keystroke::PREVIOUS_DAY, PreviousDay, outside_fields),
        KeyBinding::new("k", PreviousDay, outside_fields),
        KeyBinding::new(keystroke::NEXT_DAY, NextDay, outside_fields),
        KeyBinding::new("j", NextDay, outside_fields),
        KeyBinding::new(keystroke::SETTLE_NEXT, SettleNext, outside_fields),
        KeyBinding::new(keystroke::UNDO, Undo, context),
        KeyBinding::new(keystroke::REFRESH, Refresh, context),
        KeyBinding::new(keystroke::CLOSE, CloseBubble, context),
        KeyBinding::new("left", PreviousSegment, outside_fields),
        KeyBinding::new("right", NextSegment, outside_fields),
        KeyBinding::new("shift-left", ExtendStart, context),
        KeyBinding::new("shift-right", ExtendEnd, context),
        KeyBinding::new("alt-left", EndEarlier, context),
        KeyBinding::new("alt-right", EndLater, context),
        KeyBinding::new("alt-shift-left", StartEarlier, context),
        KeyBinding::new("alt-shift-right", StartLater, context),
        KeyBinding::new(keystroke::ADJUST_TIME, AdjustTime, context),
        KeyBinding::new(keystroke::OPEN_COMMANDS, OpenCommands, context),
        KeyBinding::new("tab", FocusNext, context),
        KeyBinding::new("shift-tab", FocusPrevious, context),
    ]
}

#[derive(Default)]
struct WeekWindow(Option<WindowHandle<WeekView>>);

impl Global for WeekWindow {}

/// Opens the week on the current day's data, or brings the open window to the front.
pub fn open(cx: &mut App) {
    if let Some(window) = cx.default_global::<WeekWindow>().0
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    let week = read(cx);
    let visible = cx
        .primary_display()
        .map(|display| display.visible_bounds().size);
    let sizes = WindowSizes::fitting(visible);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            sizes.opening,
            cx,
        ))),
        window_min_size: Some(sizes.minimum),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(language.text(Key::HistoryWindowTitle).into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| WeekView::new(week, language, window, cx))
    }) {
        Ok(window) => cx.default_global::<WeekWindow>().0 = Some(window),
        Err(error) => log::error!("cannot open the week: {error:#}"),
    }
}

/// « Voir sur la frise » of the flyout: the week, open or brought to the front, with the
/// unrecorded `intervals` of `date` selected and the bubble open.
pub fn open_on(date: jiff::civil::Date, intervals: &[UtcInterval], cx: &mut App) {
    open(cx);
    let Some(window) = cx.try_global::<WeekWindow>().and_then(|week| week.0) else {
        return;
    };
    window
        .update(cx, |view, window, cx| {
            // Closed first, so that the new range takes the focus as a click on it would.
            view.change(window, cx, |state| {
                state.close_bubble();
            });
            view.change(window, cx, |state| state.show_unrecorded(date, intervals));
        })
        .ok();
}

fn read(cx: &App) -> Option<WeekTimeline> {
    cx.global::<Today>()
        .week()
        .inspect_err(|error| log::error!("cannot read the week: {error:#}"))
        .ok()
}

/// Shows the week again in the open window, after the day changed.
pub fn reload(cx: &mut App) {
    let Some(window) = cx.try_global::<WeekWindow>().and_then(|week| week.0) else {
        return;
    };
    let week = read(cx);
    let recalculated = cx.global::<Today>().recalculated();
    let updated = window.update(cx, |view, _, cx| {
        view.reload(week, recalculated);
        if let Some(palette) = &view.palette {
            palette.set_situation(view.situation(cx), cx);
        }
        cx.notify();
    });
    if updated.is_err() {
        cx.default_global::<WeekWindow>().0 = None;
    }
}

/// Shows the result of a correction saved in « Corriger le temps… » opened from the week.
/// Returns false when the week is no longer open.
pub fn adjusted(closed: &Closed, cx: &mut App) -> bool {
    let Some(window) = cx.try_global::<WeekWindow>().and_then(|week| week.0) else {
        return false;
    };
    window
        .update(cx, |view, window, cx| {
            if let Some(state) = &mut view.state {
                state.adjusted(closed.clone());
            }
            window.activate_window();
            cx.notify();
        })
        .is_ok()
}

pub struct WeekView {
    /// `None` while no week could be read.
    state: Option<WeekState>,
    language: Language,
    tracking: Tracking,
    focus_handle: FocusHandle,
    /// The action Enter answers in the open bubble, focused when it opens.
    bubble_focus: FocusHandle,
    /// De and À, hours then minutes; De takes the focus when no answer takes Enter.
    from_fields: (FocusHandle, FocusHandle),
    to_fields: (FocusHandle, FocusHandle),
    typing: Option<bubble::Typing>,
    /// The edge of the selection a handle drags.
    dragging: Option<Edge>,
    /// Where the open day's strip was drawn, which a drag reads its position against.
    strip_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The popover a click opens comes in under it; one the keys open shows at once.
    bubble_entrance: motion::Entrance,
    /// How far into its entrance the popover is in this frame.
    bubble_time: f32,
    palette: Option<Opened>,
    days_scroll: ScrollHandle,
    /// The details just opened: the next render scrolls them into view.
    reveal_details: bool,
    /// The appointment bracket under the pointer, whose tooltip shows until the pointer leaves
    /// it or the open day or the selection changes.
    hovered_marker: Option<MarkerHover>,
    _keep_focus: Subscription,
}

impl WeekView {
    pub fn new(
        week: Option<WeekTimeline>,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        widgets::close_on_request(window, cx, |_, _| true);
        // A control that goes away with the focus, as the Details toggle on a day without a
        // summary, hands it back to the week: without it the week's keys fall to the root.
        let keep_focus = cx.on_focus_lost(window, |week, window, cx| {
            window.focus(&week.focus_handle, cx);
        });
        Self {
            state: week.map(|week| WeekState::new(week, language)),
            language,
            tracking: cx
                .try_global::<Today>()
                .map_or(Tracking::new(TrackingState::Running, true), Today::tracking),
            focus_handle,
            bubble_focus: widgets::tab_stop(cx),
            from_fields: (widgets::tab_stop(cx), widgets::tab_stop(cx)),
            to_fields: (widgets::tab_stop(cx), widgets::tab_stop(cx)),
            typing: None,
            dragging: None,
            strip_bounds: Rc::new(Cell::new(None)),
            bubble_entrance: motion::Entrance::at_rest(),
            bubble_time: 1.,
            palette: None,
            days_scroll: ScrollHandle::new(),
            reveal_details: false,
            hovered_marker: None,
            _keep_focus: keep_focus,
        }
    }

    /// The state of a view not tied to the app's day, for the screenshots.
    pub fn state_mut(&mut self) -> Option<&mut WeekState> {
        self.state.as_mut()
    }

    /// What scrolls the hours and the days, for the tests.
    #[cfg(feature = "screenshots")]
    pub fn days_scroll(&self) -> &ScrollHandle {
        &self.days_scroll
    }

    /// Shows the selection's `edge` as a handle drags it, for the screenshots.
    #[cfg(feature = "screenshots")]
    pub fn set_dragging(&mut self, edge: Option<Edge>) {
        self.dragging = edge;
    }

    /// Where the open day's strip was last drawn, for the screenshots.
    #[cfg(feature = "screenshots")]
    pub fn strip_bounds(&self) -> Option<Bounds<Pixels>> {
        self.strip_bounds.get()
    }

    /// How wide the open day's strip was last drawn.
    fn strip_width(&self) -> Option<f32> {
        self.strip_bounds
            .get()
            .map(|bounds| f32::from(bounds.size.width))
    }

    /// Brings the open popover in again, as a click opens it, for the screenshots.
    #[cfg(feature = "screenshots")]
    pub fn replay_bubble_entrance(&mut self) {
        self.bubble_entrance.reset();
    }

    /// What the status bar says of a view not tied to the app's day, for the screenshots.
    #[cfg(feature = "screenshots")]
    pub fn set_tracking(&mut self, tracking: Tracking) {
        self.tracking = tracking;
    }

    /// Ctrl+Maj+P: the command palette over the week. An open bubble closes first: it is
    /// deferred, so it would draw over the palette and keep its buttons.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_none() {
            self.change(window, cx, |state| {
                state.close_bubble();
            });
            let situation = self.situation(cx);
            let host = self.focus_handle.clone();
            self.palette = Some(Opened::new(
                situation,
                host,
                |view: &mut Self| &mut view.palette,
                window,
                cx,
            ));
            cx.notify();
        }
    }

    pub fn palette(&self) -> Option<&Entity<PaletteView>> {
        self.palette.as_ref().map(Opened::view)
    }

    fn situation(&self, cx: &App) -> Situation {
        Situation {
            host: Host::Week,
            tracking: cx
                .try_global::<Today>()
                .map_or(self.tracking, Today::tracking),
            can_correct_time: self
                .state
                .as_ref()
                .and_then(WeekState::adjust_target)
                .is_some(),
        }
    }

    fn reload(&mut self, week: Option<WeekTimeline>, recalculated: bool) {
        match (week, &mut self.state) {
            (Some(week), Some(state)) => {
                let before = state.selection();
                state.reload(week, recalculated);
                // A time half typed belongs to the range it was typed on, while it can still move.
                if state.selection() != before || state.bubble().is_none_or(|bubble| bubble.fixed) {
                    self.typing = None;
                }
            }
            (Some(week), None) => self.state = Some(WeekState::new(week, self.language)),
            (None, Some(state)) => state.load_failed(),
            (None, None) => {}
        }
    }

    fn change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut WeekState),
    ) {
        if let Some(state) = &mut self.state {
            let before = state.selection();
            change(state);
            let after = state.selection();
            if after != before {
                self.typing = None;
            }
            // A bubble opened on a segment takes the focus; one a drag or a key reshaped keeps it.
            if before.is_none() {
                self.bubble_entrance = motion::Entrance::at_rest();
            }
            match state.bubble() {
                Some(bubble)
                    if before.is_none()
                        || (after.and_then(|after| after.anchor).is_some()
                            && before.map(|before| before.anchor)
                                != after.map(|after| after.anchor)) =>
                {
                    if bubble.entered().is_some() {
                        window.focus(&self.bubble_focus, cx);
                    } else if !bubble.fixed {
                        window.focus(&self.from_fields.0, cx);
                    }
                }
                // The focus stays, unless what holds it is no longer drawn.
                Some(bubble) => {
                    let fields = [
                        &self.from_fields.0,
                        &self.from_fields.1,
                        &self.to_fields.0,
                        &self.to_fields.1,
                    ];
                    let lost = (self.bubble_focus.is_focused(window) && bubble.entered().is_none())
                        || (bubble.fixed && fields.iter().any(|field| field.is_focused(window)));
                    if lost {
                        if bubble.fixed {
                            window.focus(&self.focus_handle, cx);
                        } else {
                            window.focus(&self.from_fields.0, cx);
                        }
                    }
                }
                None if before.is_some() => {
                    self.dragging = None;
                    window.focus(&self.focus_handle, cx);
                }
                None => {}
            }
            cx.notify();
        }
    }

    /// Sends the command `make` returns to the host once this update is over, then shows its
    /// result: the host's refresh reloads the week first.
    fn run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        make: impl FnOnce(&mut WeekState) -> Option<week::WeekCommand>,
    ) {
        let Some(command) = self.state.as_mut().and_then(make) else {
            return;
        };
        window.focus(&self.focus_handle, cx);
        cx.notify();
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let mut result = Err(Key::HistoryCorrectionFailed);
            if cx.try_global::<Today>().is_some() {
                host::change(cx, |today| result = today.run_week(&command));
            }
            view.update(cx, |view, cx| {
                if let Some(state) = &mut view.state {
                    state.finished(result);
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        self.dragging = None;
        // A time half typed that moves the range moves it first: the answer waits for the bubble
        // to show it.
        if self.typing.is_some() {
            let before = self.state.as_ref().and_then(WeekState::selection);
            self.apply_typing(window, cx);
            if self.state.as_ref().and_then(WeekState::selection) != before {
                return;
            }
        }
        self.run(window, cx, |state| state.choose(answer));
    }

    /// A handle dragged to `x`: the edge moves there, snapped to a segment's edge within 6 px.
    fn drag_to(&mut self, x: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(edge), Some(bounds)) = (self.dragging, self.strip_bounds.get()) else {
            return;
        };
        let width = f32::from(bounds.size.width);
        if width <= 0. {
            return;
        }
        let fraction = f32::from(x - bounds.origin.x) / width;
        let tolerance = 6. / width;
        self.change(window, cx, |state| state.drag(edge, fraction, tolerance));
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.run(window, cx, WeekState::undo);
    }

    fn close_bubble(&mut self, _: &CloseBubble, window: &mut Window, cx: &mut Context<Self>) {
        self.change(window, cx, |state| {
            state.close_bubble();
        });
    }

    fn text(&self, key: Key) -> &'static str {
        self.language.text(key)
    }
}

/// The bracket the pointer rests on, its day's and its place among the day's brackets, and what
/// the week showed open then: its tooltip belongs to that view of the week only.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MarkerHover {
    marker: (jiff::civil::Date, usize),
    focused: jiff::civil::Date,
    selection: Option<week::Selection>,
}

impl WeekView {
    /// The bracket whose tooltip shows: none once the open day or the selection changed.
    fn shown_marker(&self) -> Option<(jiff::civil::Date, usize)> {
        let state = self.state.as_ref()?;
        self.hovered_marker
            .filter(|hover| {
                hover.focused == state.focused() && hover.selection == state.selection()
            })
            .map(|hover| hover.marker)
    }

    /// The pointer entered the bracket `marker`, or left it.
    fn hover_marker(&mut self, marker: (jiff::civil::Date, usize), hovered: bool) -> bool {
        let Some(state) = &self.state else {
            return false;
        };
        let hover = if hovered {
            Some(MarkerHover {
                marker,
                focused: state.focused(),
                selection: state.selection(),
            })
        } else {
            self.hovered_marker.filter(|hover| hover.marker != marker)
        };
        let changed = self.hovered_marker != hover;
        self.hovered_marker = hover;
        changed
    }

    /// The text of the tooltip shown, for the tests.
    #[cfg(feature = "screenshots")]
    pub fn appointment_tooltip(&self) -> Option<Vec<String>> {
        let (date, position) = self.shown_marker()?;
        let page = self.state.as_ref()?.page();
        let row = page.days.into_iter().find(|row| row.date == date)?;
        row.markers.get(position).map(|marker| marker.names.clone())
    }
}

/// A button under the open day, drawn by [`WeekView::action_button`].
struct ActionButton<F> {
    id: &'static str,
    label: Key,
    icon: Option<&'static str>,
    shortcut: Option<&'static str>,
    click: Click<F>,
}

/// What clicking an [`ActionButton`] does; a disabled one stays visible.
enum Click<F> {
    Enabled(F),
    Disabled,
}

impl<F> Click<F> {
    fn enabled_if(enabled: bool, on_click: F) -> Self {
        if enabled {
            Self::Enabled(on_click)
        } else {
            Self::Disabled
        }
    }
}

/// A button reachable with Tab and Enter, styled as GPUI Kit's ButtonLike.
fn button(
    id: impl Into<gpui::ElementId>,
    name: SharedString,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (hover, active) = (theme.element_hover, theme.element_active);
    focusable(id, name, theme)
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(active))
}

/// A list row of `lead`, then its `duration` and `action` together at its right edge: a row too
/// narrow for them wraps the two to a line of their own.
fn row_with_trailing(
    lead: impl IntoIterator<Item = AnyElement>,
    duration: impl IntoElement,
    action: impl IntoElement,
    theme: &Theme,
) -> gpui::Div {
    div()
        .min_h(size::LIST_ROW)
        .py(BASE_02)
        .flex()
        .flex_wrap()
        .items_center()
        .gap(BASE_08)
        .children(lead)
        .child(
            div()
                .ml_auto()
                .flex_none()
                .flex()
                .items_center()
                .gap(BASE_08)
                .child(
                    div()
                        .font_features(tabular_figures())
                        .text_color(theme.text_muted)
                        .child(duration),
                )
                .child(action),
        )
}

/// The single Tinted(Accent) action of a surface.
pub(crate) fn tinted(
    id: impl Into<gpui::ElementId>,
    name: SharedString,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let accent = theme.accent;
    focusable(id, name, theme)
        .bg(theme.accent_background)
        .border_color(theme.accent_border)
        .text_color(theme.accent)
        .font_weight(gpui::FontWeight::MEDIUM)
        .hover(move |style| style.border_color(accent))
}

/// What every button shares; GPUI allows one hover style per element, so each kind sets its own.
fn focusable(
    id: impl Into<gpui::ElementId>,
    name: SharedString,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let focused = theme.border_focused;
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(name)
        .tab_index(0)
        .h(size::BUTTON_MEDIUM)
        .px(BASE_08)
        .flex()
        .flex_none()
        .items_center()
        .gap(BASE_04)
        .rounded(radius::SM)
        .border_1()
        .border_color(transparent_black())
        .whitespace_nowrap()
        .cursor_pointer()
        .focus_visible(move |style| style.border_color(focused))
}

/// An action that stays visible but cannot run yet, as the design system asks. The caller adds
/// its label, after its icon if it has one.
fn disabled(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .h(size::BUTTON_MEDIUM)
        .px(BASE_08)
        .flex()
        .flex_none()
        .items_center()
        .gap(BASE_04)
        .rounded(radius::SM)
        .whitespace_nowrap()
        .text_color(theme.text_placeholder)
}

fn icon_button(
    id: &'static str,
    icon: &str,
    name: SharedString,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.text;
    button(id, name, theme)
        .group(id)
        .w(size::BUTTON_MEDIUM)
        .px_0()
        .justify_center()
        .child(
            widgets::icon(icon, theme.text_muted)
                .group_hover(id, move |style| style.text_color(hover)),
        )
}

fn tone_color(tone: DayTone, theme: &Theme) -> Hsla {
    match tone {
        DayTone::Neutral => theme.text_muted,
        DayTone::Reached => theme.success,
        DayTone::Short => theme.warning,
    }
}

/// A figure in `tone`: the text colour while neutral.
fn value_color(tone: DayTone, theme: &Theme) -> Hsla {
    match tone {
        DayTone::Neutral => theme.text,
        tone => tone_color(tone, theme),
    }
}

impl From<Severity> for Hue {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Success => Hue::Success,
            Severity::Warning => Hue::Warning,
            Severity::Error => Hue::Error,
        }
    }
}

impl WeekView {
    fn title_bar(&self, theme: &Theme) -> impl IntoElement {
        let control = |id: &'static str, icon: &str, area: WindowControlArea| {
            let hover = theme.element_hover;
            div()
                .id(id)
                .window_control_area(area)
                .w(size::WINDOW_CONTROL_WIDTH)
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(hover))
                .child(widgets::icon(icon, theme.text_muted).size(size::ICON_XS))
                .on_click(move |_, window, _| match area {
                    WindowControlArea::Min => window.minimize_window(),
                    WindowControlArea::Max => window.zoom_window(),
                    WindowControlArea::Close => window.remove_window(),
                    WindowControlArea::Drag => {}
                })
        };
        div()
            .h(size::TITLEBAR_HEIGHT)
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
                    .child(widgets::icon("calendar", theme.text_muted).size(size::ICON_SM))
                    .child(widgets::small(
                        self.text(Key::HistoryWindowTitle),
                        theme.text_muted,
                    )),
            )
            .child(control("minimize", "minus", WindowControlArea::Min))
            .child(control("maximize", "square", WindowControlArea::Max))
            .child(control("close", "x", WindowControlArea::Close))
    }

    fn header(&self, state: &WeekState, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let header = state.page().header;
        let settle = if header.can_settle {
            tinted("settle", header.settle.clone().into(), theme)
                .child(header.settle)
                .child(flyout::keys(
                    flyout::key_caps(keystroke::SETTLE_NEXT, self.language),
                    theme,
                ))
                .on_click(cx.listener(|view, _, window, cx| {
                    view.change(window, cx, WeekState::settle_next)
                }))
        } else {
            disabled("settle", header.settle.clone(), theme).child(header.settle)
        };
        let refresh = self.text(Key::HistoryRefreshAutomationPropertiesName);
        div()
            .flex()
            .items_start()
            .gap(BASE_12)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(BASE_04)
                    .child(
                        div()
                            .id("week-title")
                            .role(Role::Heading)
                            .aria_label(SharedString::from(header.title.clone()))
                            .type_style(text::HEADLINE_LG)
                            .child(header.title),
                    )
                    .child(
                        div()
                            .font_features(tabular_figures())
                            .text_color(theme.text_muted)
                            .child(flyout::readable("week-summary", header.summary)),
                    ),
            )
            .child(settle)
            .child(
                icon_button(
                    "refresh",
                    "refresh-cw",
                    flyout::shortcut_name(refresh, keystroke::REFRESH, self.language),
                    theme,
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx)),
            )
    }

    fn info_bar(
        &self,
        state: &WeekState,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let bar = state.info_bar()?;
        let hue = Hue::from(bar.severity);
        let HueColors {
            background,
            border,
            foreground,
        } = hue.colors(theme);
        let icon = match bar.severity {
            Severity::Error => "circle-x",
            Severity::Success | Severity::Warning => hue.icon(),
        };
        let outlined = |button: gpui::Stateful<gpui::Div>| button.border_color(theme.border);
        let undo = self.text(Key::HistoryResultUndoContent);
        let close = self.text(Key::CommandErrorDialogCloseButton);
        // An offer's lines keep the icon and the buttons level with the message.
        let offered = bar.offer.is_some();
        Some(
            div()
                .id("result")
                .role(Role::Status)
                .aria_label(SharedString::from(bar.message.clone()))
                .flex()
                .when(offered, |bar| bar.items_start())
                .when(!offered, |bar| bar.items_center())
                .gap(BASE_08)
                .py(BASE_08)
                .px(BASE_12)
                .rounded(radius::MD)
                .border_1()
                .border_color(border)
                .bg(background)
                .child(widgets::icon(icon, foreground))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(bar.message)
                        .children(bar.offer.map(|offer| self.offer(offer, theme, cx))),
                )
                .children(bar.can_undo.then(|| {
                    outlined(button(
                        "undo",
                        flyout::shortcut_name(undo, keystroke::UNDO, self.language),
                        theme,
                    ))
                    .child(undo)
                    .child(flyout::keys(
                        flyout::key_caps(keystroke::UNDO, self.language),
                        theme,
                    ))
                    .on_click(cx.listener(|view, _, window, cx| view.undo(&Undo, window, cx)))
                }))
                .children(bar.next.map(|next| {
                    outlined(button("next", next.clone().into(), theme))
                        .child(next)
                        .child(flyout::keys(
                            flyout::key_caps(keystroke::SETTLE_NEXT, self.language),
                            theme,
                        ))
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.change(window, cx, WeekState::settle_next)
                        }))
                }))
                .children(bar.dismissible.then(|| {
                    icon_button("dismiss", "x", close.into(), theme).on_click(cx.listener(
                        |view, _, window, cx| view.change(window, cx, WeekState::dismiss_result),
                    ))
                }))
                .into_any_element(),
        )
    }

    /// « Compter chaque jeudi… » under a correction the same as on the previous days of its
    /// weekday, and « Ne plus proposer pour le jeudi ».
    fn offer(
        &self,
        offer: week::ProposalOffer,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(BASE_06)
            .child(
                div()
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .child(offer.note),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(BASE_08)
                    .child(
                        button("accept-proposal", offer.accept.clone().into(), theme)
                            .border_color(theme.border)
                            .gap(BASE_06)
                            .text_color(theme.accent)
                            .child(widgets::icon("calendar", theme.accent).size(size::ICON_SM))
                            .child(offer.accept)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.answer_proposal(true, window, cx)
                            })),
                    )
                    .child(
                        button("dismiss-proposal", offer.dismiss.clone().into(), theme)
                            .text_color(theme.text_muted)
                            .underline()
                            .child(offer.dismiss)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.answer_proposal(false, window, cx)
                            })),
                    ),
            )
    }

    /// Opens the proposed appointment in « Paramètres », or turns it down for good.
    fn answer_proposal(&mut self, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(proposal) = self
            .state
            .as_ref()
            .and_then(|state| state.proposal())
            .cloned()
        else {
            return;
        };
        // The offer stays while the dialog of « Paramètres » holds them.
        let answered = if accept {
            settings_view::open_with_proposal(&proposal, cx)
        } else {
            settings_view::dismiss_proposal(&proposal, cx)
        };
        if answered {
            self.change(window, cx, WeekState::answer_proposal);
        }
    }

    fn adjust_button(&self, enabled: bool, theme: &Theme) -> AnyElement {
        self.action_button(
            ActionButton {
                id: "adjust-time",
                label: Key::FlyoutAdjustTime,
                icon: Some("pencil"),
                shortcut: Some(keystroke::ADJUST_TIME),
                click: Click::enabled_if(
                    enabled,
                    |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                        window.dispatch_action(Box::new(AdjustTime), cx)
                    },
                ),
            },
            theme,
        )
    }

    /// A button under the open day, with its icon and its shortcut's caps if any.
    fn action_button<F>(&self, action: ActionButton<F>, theme: &Theme) -> AnyElement
    where
        F: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    {
        let label = self.text(action.label);
        let icon_color = match action.click {
            Click::Enabled(_) => theme.text_muted,
            Click::Disabled => theme.text_placeholder,
        };
        let icon = action
            .icon
            .map(|icon| widgets::icon(icon, icon_color).size(size::ICON_SM));
        let caps = action
            .shortcut
            .map(|shortcut| flyout::keys(flyout::key_caps(shortcut, self.language), theme));
        let control = match action.click {
            Click::Enabled(on_click) => {
                let name = action.shortcut.map_or(label.into(), |shortcut| {
                    flyout::shortcut_name(label, shortcut, self.language)
                });
                button(action.id, name, theme)
                    .bg(theme.element)
                    .border_color(theme.border_variant)
                    .on_click(on_click)
            }
            Click::Disabled => disabled(action.id, label, theme),
        };
        control
            .children(icon)
            .child(label)
            .children(caps)
            .into_any_element()
    }

    fn legend(&self, page: &week::WeekPage, theme: &Theme) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .gap_x(BASE_16)
            .gap_y(BASE_08)
            .type_style(text::LABEL_SM)
            .text_color(theme.text_muted)
            .children(page.legend().into_iter().map(|(mark, key)| {
                let glyph = match mark {
                    LegendMark::Segment(kind) => swatch(Some(kind), theme).into_any_element(),
                    LegendMark::Future => swatch(None, theme).into_any_element(),
                    LegendMark::Appointment => bracket_glyph(theme).into_any_element(),
                    LegendMark::Now => div()
                        .w(size::NOW_LINE_WIDTH)
                        .h(size::ICON_XS)
                        .bg(theme.tl_now)
                        .into_any_element(),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(BASE_06)
                    .child(glyph)
                    .child(self.text(key))
            }))
    }

    fn status_bar(&self, theme: &Theme) -> impl IntoElement {
        let hint = |id: &'static str, caps: Vec<SharedString>, label: Key| {
            let label = self.text(label);
            div()
                .id(id)
                .role(Role::Group)
                .aria_label(label)
                .flex()
                .items_center()
                .gap(BASE_06)
                .child(flyout::keys(caps, theme))
                .child(label)
        };
        let days = [keystroke::PREVIOUS_DAY, keystroke::NEXT_DAY]
            .iter()
            .flat_map(|keystroke| flyout::key_caps(keystroke, self.language))
            .collect();
        div()
            .h(size::STATUSBAR_HEIGHT)
            .flex_none()
            .px(BASE_08)
            .flex()
            .items_center()
            .gap(BASE_12)
            .bg(theme.background)
            .border_t_1()
            .border_color(theme.border_variant)
            .type_style(text::LABEL_SM)
            .text_color(theme.text_muted)
            .whitespace_nowrap()
            .child(flyout::tracking_status(
                self.tracking.status(),
                self.language,
                theme,
            ))
            .child(div().flex_1())
            .child(hint("change-day", days, Key::HistoryStatusChangeDay))
            .child(hint(
                "next-pending",
                flyout::key_caps(keystroke::SETTLE_NEXT, self.language),
                Key::HistoryStatusNextPending,
            ))
    }
}

impl Render for WeekView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        if let Some(today) = cx.try_global::<Today>() {
            self.tracking = today.tracking();
        }
        let mut main = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(BASE_16)
            .py(BASE_20)
            .px(BASE_24);
        self.bubble_time = if self
            .state
            .as_ref()
            .is_some_and(|state| state.selection().is_some())
        {
            self.bubble_entrance.time(motion::MENU, window, cx)
        } else {
            1.
        };
        match self.state.clone() {
            Some(state) => {
                main = main
                    .child(self.header(&state, &theme, cx))
                    .children(self.info_bar(&state, &theme, cx))
                    .child(self.days(&state, &theme, window, cx))
                    .child(self.legend(&state.page(), &theme));
            }
            None => {
                main = main.child(
                    div()
                        .py(BASE_08)
                        .px(BASE_12)
                        .rounded(radius::MD)
                        .border_1()
                        .border_color(theme.error_border)
                        .bg(theme.error_background)
                        .child(self.text(Key::HistoryLoadFailed)),
                );
            }
        }
        div()
            .id("week")
            .role(Role::Window)
            .aria_label(self.text(Key::HistoryWindowTitle))
            // The open palette takes the keys, so that N, P, J and K type in its query.
            .when(self.palette.is_none(), |week| week.key_context(KEY_CONTEXT))
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|view, _: &OpenCommands, window, cx| view.open_palette(window, cx)),
            )
            .on_action(cx.listener(|view, _: &PreviousDay, window, cx| {
                view.change(window, cx, |state| state.move_focus(-1))
            }))
            .on_action(cx.listener(|view, _: &NextDay, window, cx| {
                view.change(window, cx, |state| state.move_focus(1))
            }))
            .on_action(cx.listener(|view, _: &SettleNext, window, cx| {
                view.change(window, cx, WeekState::settle_next)
            }))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::close_bubble))
            .on_action(cx.listener(|view, _: &PreviousSegment, window, cx| {
                view.change(window, cx, |state| state.step(-1))
            }))
            .on_action(cx.listener(|view, _: &NextSegment, window, cx| {
                view.change(window, cx, |state| state.step(1))
            }))
            .on_action(cx.listener(|view, _: &ExtendStart, window, cx| {
                view.change(window, cx, |state| state.extend(Edge::Start))
            }))
            .on_action(cx.listener(|view, _: &ExtendEnd, window, cx| {
                view.change(window, cx, |state| state.extend(Edge::End))
            }))
            .on_action(cx.listener(|view, _: &StartEarlier, window, cx| {
                view.change(window, cx, |state| state.nudge(Edge::Start, -1))
            }))
            .on_action(cx.listener(|view, _: &StartLater, window, cx| {
                view.change(window, cx, |state| state.nudge(Edge::Start, 1))
            }))
            .on_action(cx.listener(|view, _: &EndEarlier, window, cx| {
                view.change(window, cx, |state| state.nudge(Edge::End, -1))
            }))
            .on_action(cx.listener(|view, _: &EndLater, window, cx| {
                view.change(window, cx, |state| state.nudge(Edge::End, 1))
            }))
            .on_mouse_move(
                cx.listener(|view, event: &gpui::MouseMoveEvent, window, cx| {
                    if view.dragging.is_none() {
                        return;
                    }
                    // Released where the root doesn't hear it: over the popover or outside.
                    if event.pressed_button != Some(MouseButton::Left) {
                        view.dragging = None;
                        cx.notify();
                        return;
                    }
                    view.drag_to(event.position.x, window, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.dragging.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(Self::adjust_time))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.editor)
            .text_color(theme.text)
            .type_style(text::LABEL_MD)
            .child(self.title_bar(&theme))
            .child(main)
            .child(self.status_bar(&theme))
            .children(
                self.palette
                    .as_ref()
                    .map(|palette| palette.overlay(&theme).top(size::TITLEBAR_HEIGHT)),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyBindingContextPredicate, Keystroke};

    use super::*;

    #[test]
    fn the_arrows_move_the_selection_with_their_modifiers() {
        let week = KeyBindingContextPredicate::parse(KEY_CONTEXT).unwrap();
        let bound = |typed: &str| {
            let typed = [Keystroke::parse(typed).unwrap()];
            bindings()
                .iter()
                .filter(|binding| binding.match_keystrokes(&typed) == Some(false))
                .map(|binding| (binding.action().name(), binding.predicate()))
                .collect::<Vec<_>>()
        };
        let outside_fields = KeyBindingContextPredicate::parse(OUTSIDE_FIELDS).unwrap();
        for (typed, action) in [
            ("left", "week::PreviousSegment"),
            ("right", "week::NextSegment"),
        ] {
            assert_eq!(
                bound(typed),
                [(action, Some(outside_fields.clone().into()))],
                "{typed}"
            );
        }
        for (typed, action) in [
            ("shift-left", "week::ExtendStart"),
            ("shift-right", "week::ExtendEnd"),
            ("alt-left", "week::EndEarlier"),
            ("alt-right", "week::EndLater"),
            ("alt-shift-left", "week::StartEarlier"),
            ("alt-shift-right", "week::StartLater"),
        ] {
            assert_eq!(
                bound(typed),
                [(action, Some(week.clone().into()))],
                "{typed}"
            );
        }
    }

    /// In « De » and « À », the arrows step the field and letters type nothing: none of them
    /// reaches the week.
    #[test]
    fn the_fields_keep_their_arrows_and_letters() {
        let field = [
            gpui::KeyContext::parse(KEY_CONTEXT).unwrap(),
            gpui::KeyContext::parse(FIELD_CONTEXT).unwrap(),
        ];
        let week = [gpui::KeyContext::parse(KEY_CONTEXT).unwrap()];
        let bound = |typed: &str, contexts: &[gpui::KeyContext]| {
            let typed = [Keystroke::parse(typed).unwrap()];
            bindings().iter().any(|binding| {
                binding.match_keystrokes(&typed) == Some(false)
                    && binding
                        .predicate()
                        .is_none_or(|predicate| predicate.depth_of(contexts).is_some())
            })
        };
        for typed in ["up", "down", "left", "right", "j", "k", "n"] {
            assert!(!bound(typed, &field), "{typed}");
            assert!(bound(typed, &week), "{typed}");
        }
        assert!(bound("escape", &field));
    }
}
