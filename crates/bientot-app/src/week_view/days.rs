//! The days of the week window: the hours, then one card per weekday with its timeline drawn
//! with `canvas()`, the open one with its focus.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, BorderStyle, Bounds, ClickEvent, Context, Corners, Edges, Hsla,
    IntoElement, Pixels, Role, SharedString, TextAlign, Window, anchored, canvas, deferred, div,
    fill, linear_color_stop, linear_gradient, pattern_slash, point, prelude::*, px, quad, relative,
    size, transparent_black,
};

use super::{Layout, WeekView, button, row_with_trailing, tone_color, value_color};
use crate::strings::{Key, Language};
use crate::theme::spacing::*;
use crate::theme::{
    Theme, TypeStyle, TypeStyled as _, motion, radius, size, tabular_figures, text,
};
use crate::week::{
    self, DayRow, DayTone, MarkerEnd, MarkerView, SegmentKind, SegmentRole, SegmentView, WeekState,
};
use crate::widgets;

impl WeekView {
    /// The hours, then one card per day, in [`Self::days_scroll`], under a shade while it is
    /// scrolled down.
    pub(super) fn days(
        &mut self,
        state: &WeekState,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = state.page();
        let hours = div()
            .flex()
            .gap(BASE_16)
            .px(BASE_12)
            .child(div().w(size::DAY_COLUMN).flex_none())
            .child(div().flex_1().min_w_0().child(axis(
                page.start_hour,
                page.end_hour,
                self.language,
                theme,
            )))
            .child(div().w(size::DAY_COLUMN).flex_none());
        let layout = Layout::for_width(window.viewport_size().width);
        let mut details = state.details().map(|details| {
            self.details(details, layout, theme, window, cx)
                .into_any_element()
        });
        let reveal = std::mem::take(&mut self.reveal_details) && details.is_some();
        let can_adjust = state.adjust_target().is_some();
        let mut items = vec![hours.into_any_element()];
        let mut focused_card = None;
        for (index, row) in page.days.into_iter().enumerate() {
            let details = if row.focused {
                focused_card = Some(items.len());
                details.take()
            } else {
                None
            };
            let focus = row.focus.clone().map(|focus| {
                self.focus(focus, can_adjust, details, layout, theme, cx)
                    .into_any_element()
            });
            items.push(
                self.day_row(index, row, focus, state, theme, cx)
                    .into_any_element(),
            );
        }
        if reveal && let Some(card) = focused_card {
            self.days_scroll.scroll_to_item(card);
        }
        let list = div()
            .id("days")
            .role(Role::Group)
            .aria_label(self.text(Key::HistoryTimelineAutomationPropertiesName))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.days_scroll)
            .flex()
            .flex_col()
            .gap(BASE_04)
            .pb(BASE_08)
            .children(items);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(list)
            .child(widgets::scrolled_edge(self.days_scroll.clone(), theme))
    }

    /// The card of one day; the open one also holds `focus`, its [`Self::focus`].
    fn day_row(
        &self,
        index: usize,
        row: DayRow,
        focus: Option<AnyElement>,
        state: &WeekState,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let date = row.date;
        let select = cx.listener(move |view, _, window, cx| {
            view.change(window, cx, |state| state.select(date))
        });
        let status_icon = match row.tone {
            DayTone::Reached => Some("circle-check"),
            DayTone::Short => Some("triangle-alert"),
            DayTone::Neutral if row.is_today => Some("clock"),
            DayTone::Neutral => None,
        };
        let status_color = tone_color(row.tone, theme);
        let label = button(("day", index), row.accessible_name.clone().into(), theme)
            .w(size::DAY_COLUMN)
            .h_auto()
            .py(BASE_02)
            // A status too long for the column wraps within it.
            .whitespace_normal()
            .flex_col()
            .items_start()
            .gap_0()
            .child(
                div()
                    .type_style(text::LABEL_STRONG)
                    .child(row.label.clone()),
            )
            .child(
                div()
                    .max_w_full()
                    .flex()
                    .items_start()
                    .gap(BASE_04)
                    .type_style(text::LABEL_SM)
                    .text_color(status_color)
                    // The icon stays on the first line of a status that wraps.
                    .children(status_icon.map(|name| {
                        div()
                            .h(text::LABEL_SM.line_height)
                            .flex()
                            .items_center()
                            .child(widgets::icon(name, status_color).size(size::ICON_XS))
                    }))
                    .child(div().min_w_0().child(row.status.clone())),
            )
            .children(row.detail.clone().map(|detail| {
                div()
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .font_features(tabular_figures())
                    .child(detail)
            }))
            .on_click(select);

        let height = if row.focused {
            size::TIMELINE_ROW_FOCUSED
        } else {
            size::TIMELINE_ROW
        };
        let bubble = row
            .focused
            .then(|| state.bubble())
            .flatten()
            .filter(|bubble| bubble.selection.date == row.date);
        // What lies under the open popover dims with the rest of the day.
        let under = if bubble.is_some() {
            1. - 0.55 * motion::ease_out()(self.bubble_time)
        } else {
            1.
        };
        // The segment clicked holds the popover open on its range; once the range moved, the
        // segment it starts in.
        let holder = bubble.as_ref().and_then(|bubble| {
            bubble.selection.anchor.or_else(|| {
                row.segments
                    .iter()
                    .rfind(|segment| segment.id.start <= bubble.selection.range.start())
                    .filter(|segment| segment.role == SegmentRole::Button)
                    .map(|segment| segment.id)
            })
        });
        // The strip is painted, so each segment gets a node of its own for screen readers.
        let overlays = row
            .segments
            .iter()
            .enumerate()
            .map(|(position, segment)| {
                let node = div()
                    .id(("segment", index * 1000 + position))
                    .aria_label(SharedString::from(segment.accessible_name.clone()))
                    .absolute()
                    .top(-BASE_02)
                    .bottom(-BASE_02)
                    .left(relative(segment.start))
                    .w(relative(segment.length))
                    .min_w(px(week::MINIMUM_SEGMENT_WIDTH));
                match segment.role {
                    // AccessKit names a Label from its value, not its label.
                    SegmentRole::Label => node
                        .role(Role::Label)
                        .aria_value(SharedString::from(segment.accessible_name.clone())),
                    SegmentRole::Button => {
                        let id = segment.id;
                        let focused = theme.border_focused;
                        let opened = holder == Some(id);
                        node.role(Role::Button)
                            .aria_expanded(opened)
                            .tab_index(0)
                            .rounded(radius::XS)
                            .border_2()
                            .border_color(transparent_black())
                            .cursor_pointer()
                            .focus_visible(move |style| style.border_color(focused))
                            .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                                let opening = view
                                    .state
                                    .as_ref()
                                    .is_some_and(|state| state.selection().is_none());
                                view.change(window, cx, |state| state.open(id));
                                // Only a click brings the popover in under the pointer.
                                if opening && !event.is_keyboard() {
                                    view.bubble_entrance.reset();
                                }
                            }))
                    }
                }
            })
            .collect::<Vec<_>>();
        // Each appointment's bracket is painted too: a node over it names it and shows its
        // tooltip, as wide as the bracket and taller, to be easier to point at.
        let gap = marker_gap(row.focused);
        let markers = row.markers.iter().enumerate().map(|(position, marker)| {
            let key = (date, position);
            let hovered = self.shown_marker() == Some(key);
            let hit = div()
                .id(("appointment", index * 1000 + position))
                .role(Role::Label)
                .aria_value(SharedString::from(marker.accessible_name.clone()))
                .size_full()
                .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                    if view.hover_marker(key, *hovered) {
                        cx.notify();
                    }
                }))
                .children(hovered.then(|| marker_tooltip(marker, theme)));
            div()
                .absolute()
                .top(
                    -(gap
                        + size::APPOINTMENT_MARKER_HEIGHT
                        + (size::APPOINTMENT_MARKER_HIT - size::APPOINTMENT_MARKER_HEIGHT) / 2.),
                )
                .h(size::APPOINTMENT_MARKER_HIT)
                .left(relative(marker.start))
                .w(relative(marker.length))
                .min_w(px(week::MINIMUM_SEGMENT_WIDTH))
                .when(marker.right == MarkerEnd::Now, |span| {
                    span.pr(now_clearance())
                })
                .child(hit)
        });
        let strip_area = div()
            .id(("strip", index))
            .relative()
            .w_full()
            .h(height)
            .child(strip(
                &row,
                row.focused.then(|| self.strip_bounds.clone()),
                bubble
                    .as_ref()
                    .map(|bubble| (bubble.start, bubble.start + bubble.length)),
                theme,
            ))
            .children(overlays)
            .children(markers)
            .children(
                bubble
                    .as_ref()
                    .map(|bubble| self.selection(bubble, theme, cx)),
            )
            .children(bubble.map(|bubble| self.bubble(bubble, theme, cx)))
            .when(!row.focused, |strip| {
                strip
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.change(window, cx, |state| state.select(date))
                    }))
            });

        let total = div()
            .w(size::DAY_COLUMN)
            .flex_none()
            .flex()
            .flex_col()
            .items_end()
            .gap(BASE_02)
            .whitespace_nowrap()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .font_features(tabular_figures())
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(row.total.clone()),
                    )
                    .child(
                        div()
                            .type_style(text::LABEL_SM)
                            .text_color(theme.text_muted)
                            .child(format!("\u{a0}{}", row.target)),
                    ),
            )
            .children(row.badge.clone().map(|badge| {
                div()
                    .h(px(18.))
                    .px(BASE_06)
                    .rounded(radius::MD)
                    .border_1()
                    .border_color(theme.warning_border)
                    .bg(theme.warning_background)
                    .text_color(theme.warning)
                    .type_style(text::LABEL_SM)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(badge)
            }));

        // The open day's card holds its brackets: room above its strip for them, clear of its
        // top edge.
        let bracket_room = (row.focused && !row.markers.is_empty()).then(|| {
            size::APPOINTMENT_MARKER_INSET
                + size::APPOINTMENT_MARKER_HEIGHT
                + size::APPOINTMENT_MARKER_GAP_FOCUSED
                - CARD_PADDING
        });
        let line = div()
            .flex()
            .items_center()
            .gap(BASE_16)
            .child(label)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when_some(bracket_room, |column, room| column.pt(room))
                    .child(strip_area),
            )
            .child(total);
        div()
            .flex()
            .flex_col()
            .gap(BASE_12)
            .py(CARD_PADDING)
            .px(BASE_12)
            .rounded(radius::MD)
            .when(row.focused, |row| row.bg(theme.surface))
            .child(line)
            .children(focus.map(|focus| div().opacity(under).child(focus)))
    }

    /// What the open day shows under its strip; `details`, the panel its toggle opened.
    fn focus(
        &self,
        focus: week::DayFocus,
        can_adjust: bool,
        details: Option<AnyElement>,
        layout: Layout,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settle = self.text(Key::HistoryPendingSettleContent);
        let list = (!focus.pending.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(BASE_02)
                .child(widgets::caps(&focus.pending_title, theme))
                .children(focus.pending.iter().enumerate().map(|(position, item)| {
                    let id = item.id;
                    let lead = div()
                        .flex()
                        .items_center()
                        .gap(BASE_08)
                        .child(swatch(Some(item.kind), theme))
                        .child(
                            div()
                                .font_features(tabular_figures())
                                .child(item.range.clone()),
                        )
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .child(format!("· {}", item.kind_name)),
                        );
                    let settle = button(
                        ("settle-item", position),
                        item.settle_name.clone().into(),
                        theme,
                    )
                    .bg(theme.element)
                    .border_color(theme.border_variant)
                    .child(settle)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.change(window, cx, |state| state.open(id))
                    }));
                    row_with_trailing(
                        [lead.into_any_element()],
                        item.duration.clone(),
                        settle,
                        theme,
                    )
                    .pl(BASE_08)
                    .pr(BASE_04)
                    .rounded(radius::SM)
                    .bg(theme.element)
                }))
        });
        // Two columns, labels then values, each line read as one sentence.
        let summary = div()
            .min_w(size::DAY_COLUMN)
            .flex_none()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .type_style(text::LABEL_SM)
            .whitespace_nowrap()
            .children(
                focus
                    .summary
                    .into_iter()
                    .enumerate()
                    .map(|(position, line)| {
                        div()
                            .id(("summary", position))
                            .role(Role::Group)
                            .aria_label(SharedString::from(line.accessible_name))
                            .flex()
                            .justify_between()
                            .gap(BASE_12)
                            .child(div().text_color(theme.text_muted).child(line.label))
                            .child(
                                div()
                                    .text_right()
                                    .font_features(tabular_figures())
                                    .text_color(value_color(line.tone, theme))
                                    .child(line.value),
                            )
                    }),
            );
        div()
            .flex()
            .items_start()
            .gap(BASE_16)
            .map(|focus| match layout {
                Layout::Wide => focus.child(div().w(size::DAY_COLUMN).flex_none()),
                // In line with the text of the day's label, inside its border and padding.
                Layout::Narrow => focus.pl(BASE_08 + px(1.)),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(BASE_08)
                    .children(list)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(BASE_08)
                            .child(self.adjust_button(can_adjust, theme))
                            .child(self.declare_button(can_adjust, theme, cx)),
                    )
                    // On a row of its own, over the panel it opens, whatever the width and the
                    // language leave beside the buttons.
                    .children(
                        self.details_toggle(theme, cx)
                            .map(|toggle| div().flex().child(toggle)),
                    )
                    .children(details),
            )
            .child(summary)
    }
}

/// The fill of a segment of `kind`, also its legend swatch.
pub(crate) fn segment_quad(
    kind: SegmentKind,
    bounds: Bounds<Pixels>,
    theme: &Theme,
) -> Vec<gpui::PaintQuad> {
    let shape = |background: gpui::Background, border: Option<(Hsla, BorderStyle)>| {
        let (border_color, style) = border.unwrap_or((transparent_black(), BorderStyle::Solid));
        quad(
            bounds,
            Corners::all(radius::XS),
            background,
            px(if border.is_some() { 1. } else { 0. }),
            border_color,
            style,
        )
    };
    match kind {
        SegmentKind::Work => vec![shape(theme.tl_work.into(), None)],
        SegmentKind::Manual => vec![shape(theme.tl_manual.into(), None)],
        SegmentKind::Pause => vec![shape(theme.tl_pause.into(), None)],
        SegmentKind::NotWorked => vec![shape(
            theme.tl_not_worked.into(),
            Some((theme.text_placeholder, BorderStyle::Solid)),
        )],
        SegmentKind::Gap | SegmentKind::Sleep => vec![
            shape(theme.editor.into(), None),
            shape(theme.warning_background.into(), None),
            shape(
                pattern_slash(theme.tl_gap_stripe, 1.5, 6.),
                Some((theme.warning, BorderStyle::Solid)),
            ),
        ],
        SegmentKind::Review => vec![shape(
            theme.tl_review.into(),
            Some((theme.warning, BorderStyle::Dashed)),
        )],
    }
}

/// The style of the text painted by hand: the segments' labels and the hour axis.
const TICK_TEXT: TypeStyle = text::WT_VALUE_SM;

fn paint_text(
    text: &str,
    origin: gpui::Point<Pixels>,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    let line = widgets::shape_text(text, TICK_TEXT, color, window);
    line.paint(
        origin,
        TICK_TEXT.line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    )
    .ok();
    line.width
}

/// The strip of one day: its track, its segments, the open day's labels, the « maintenant »
/// line and the brackets of its appointments above it, painted by hand as the design system's
/// Frise. `bounds` keeps where it was painted.
/// While a range is `selected`, only the segments starting in it keep their label: the others
/// are dimmed, under its handles.
fn strip(
    row: &DayRow,
    bounds: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
    selected: Option<(f32, f32)>,
    theme: &Theme,
) -> impl IntoElement {
    let segments = row.segments.clone();
    let markers = row.markers.clone();
    let (focused, future, now) = (row.focused, row.future, row.now);
    let theme = theme.clone();
    canvas(
        |_, _, _| {},
        move |painted, _, window, cx| {
            if let Some(bounds) = &bounds {
                bounds.set(Some(painted));
            }
            let bounds = painted;
            let track = if future {
                quad(
                    bounds,
                    Corners::all(radius::XS),
                    transparent_black(),
                    px(1.),
                    theme.text_placeholder,
                    BorderStyle::Dashed,
                )
            } else if focused {
                quad(
                    bounds,
                    Corners::all(radius::XS),
                    theme.tl_track_focused,
                    px(1.),
                    theme.border_variant,
                    BorderStyle::Solid,
                )
            } else {
                fill(bounds, theme.tl_track).corner_radii(Corners::all(radius::XS))
            };
            window.paint_quad(track);
            let width = f32::from(bounds.size.width);
            for segment in &segments {
                let (x, w) = week::place(segment.start, segment.length, width);
                let placed = Bounds::new(
                    point(bounds.origin.x + px(x), bounds.origin.y),
                    size(px(w), bounds.size.height),
                );
                for shape in segment_quad(segment.kind, placed, &theme) {
                    window.paint_quad(shape);
                }
                let labelled = selected
                    .is_none_or(|(start, end)| segment.start >= start && segment.start < end);
                if focused && labelled {
                    paint_label(segment, placed, &theme, window, cx);
                }
            }
            if let Some(now) = now {
                let x = bounds.origin.x + px(now * width) - size::NOW_LINE_WIDTH / 2.;
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(x, bounds.origin.y - size::NOW_LINE_OVERHANG),
                            size(
                                size::NOW_LINE_WIDTH,
                                bounds.size.height + size::NOW_LINE_OVERHANG * 2.,
                            ),
                        ),
                        theme.tl_now,
                    )
                    .corner_radii(Corners::all(size::NOW_LINE_WIDTH / 2.)),
                );
            }
            let gap = marker_gap(focused);
            for marker in &markers {
                paint_marker(marker, bounds, gap, &theme, window);
            }
        },
    )
    .size_full()
}

/// Inside a day's card, above and below what it holds.
const CARD_PADDING: Pixels = BASE_08;

/// Between a day's strip and the bracket over it.
fn marker_gap(focused: bool) -> Pixels {
    if focused {
        size::APPOINTMENT_MARKER_GAP_FOCUSED
    } else {
        size::APPOINTMENT_MARKER_GAP
    }
}

/// How far short of now a bracket cut by now stops: past half the line, then its clearance.
fn now_clearance() -> Pixels {
    size::NOW_LINE_WIDTH / 2. + size::APPOINTMENT_MARKER_NOW_CLEARANCE
}

/// The bracket over `marker`, `gap` above the strip painted in `strip`: a line with a tick at
/// each planned end. An end cut by the axis is left open; one cut by now fades out short of the
/// « maintenant » line.
fn paint_marker(
    marker: &MarkerView,
    strip: Bounds<Pixels>,
    gap: Pixels,
    theme: &Theme,
    window: &mut Window,
) {
    let (x, w) = week::place(marker.start, marker.length, f32::from(strip.size.width));
    let left = strip.origin.x + px(x);
    let mut right = left + px(w);
    let fade = if marker.right == MarkerEnd::Now {
        right = (right - now_clearance()).max(left + size::APPOINTMENT_MARKER_STROKE);
        size::APPOINTMENT_MARKER_FADE.min((right - left) / 2.)
    } else {
        px(0.)
    };
    let top = strip.origin.y - gap - size::APPOINTMENT_MARKER_HEIGHT;
    let color = theme.text_muted;
    let tick = |end: MarkerEnd, width| {
        if end == MarkerEnd::Planned {
            width
        } else {
            px(0.)
        }
    };
    window.paint_quad(quad(
        Bounds::new(
            point(left, top),
            size(right - fade - left, size::APPOINTMENT_MARKER_HEIGHT),
        ),
        Corners {
            top_left: tick(marker.left, radius::XS),
            top_right: tick(marker.right, radius::XS),
            ..Corners::default()
        },
        transparent_black(),
        Edges {
            top: size::APPOINTMENT_MARKER_STROKE,
            left: tick(marker.left, size::APPOINTMENT_MARKER_STROKE),
            right: tick(marker.right, size::APPOINTMENT_MARKER_STROKE),
            bottom: px(0.),
        },
        color,
        BorderStyle::Solid,
    ));
    if fade > px(0.) {
        window.paint_quad(fill(
            Bounds::new(
                point(right - fade, top),
                size(fade, size::APPOINTMENT_MARKER_STROKE),
            ),
            linear_gradient(
                90.,
                linear_color_stop(color, 0.),
                linear_color_stop(color.opacity(0.), 1.),
            ),
        ));
    }
}

/// The names of `marker`'s appointments, one per line, above its bracket: from its start
/// rightwards over the strip's first half, from its end leftwards over the second, so that it
/// stays over the strip and off the day's total, and within the window.
fn marker_tooltip(marker: &MarkerView, theme: &Theme) -> impl IntoElement {
    let rightwards = marker.start + marker.length / 2. < 0.5;
    // From the top of the area the pointer finds the bracket on, centred on the bracket.
    let above = size::APPOINTMENT_TOOLTIP_GAP
        - (size::APPOINTMENT_MARKER_HIT - size::APPOINTMENT_MARKER_HEIGHT) / 2.;
    div()
        .absolute()
        .top(-above)
        .map(|anchor| {
            if rightwards {
                anchor.left_0()
            } else {
                anchor.right_0()
            }
        })
        .child(
            deferred(
                anchored()
                    .anchor(if rightwards {
                        Anchor::BottomLeft
                    } else {
                        Anchor::BottomRight
                    })
                    .snap_to_window_with_margin(BASE_08)
                    .child(widgets::tooltip_box(theme).children(marker.names.clone())),
            )
            .with_priority(2),
        )
}

fn paint_label(
    segment: &SegmentView,
    placed: Bounds<Pixels>,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(label) = week::segment_label(
        segment.kind,
        &segment.start_time,
        segment.word,
        f32::from(placed.size.width),
    ) else {
        return;
    };
    let line_height = TICK_TEXT.line_height;
    let origin = point(
        placed.origin.x + BASE_08,
        placed.origin.y + (placed.size.height - line_height) / 2.,
    );
    let color = match segment.kind {
        SegmentKind::Work | SegmentKind::Manual => theme.tl_on_fill,
        SegmentKind::NotWorked => theme.text_muted,
        _ => theme.text,
    };
    if matches!(
        segment.kind,
        SegmentKind::Gap | SegmentKind::Sleep | SegmentKind::Review
    ) {
        // The word sits on an opaque tag, readable over the hatching.
        let tag = Bounds::new(
            point(origin.x - BASE_04, origin.y),
            size(
                widgets::text_width(label, TICK_TEXT, window) + BASE_08,
                line_height,
            ),
        );
        window.paint_quad(fill(tag, theme.editor).corner_radii(Corners::all(radius::XS)));
    }
    paint_text(label, origin, color, window, cx);
}

/// The hour axis above the strips, its step chosen from the width it gets. Each label is
/// centred on its tick; the first and last reach into the gaps beside the strip.
fn axis(start_hour: i8, end_hour: i8, language: Language, theme: &Theme) -> impl IntoElement {
    let color = theme.text_muted;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let width = f32::from(bounds.size.width);
            let widest = f32::from(widgets::text_width(
                &language.format(Key::TimelineHourFormat, &[&0]),
                TICK_TEXT,
                window,
            ));
            for (at, label) in week::axis(start_hour, end_hour, width, widest, language) {
                let label_width = widgets::text_width(&label, TICK_TEXT, window);
                let x = bounds.origin.x + px(at * width) - label_width / 2.;
                paint_text(&label, point(x, bounds.origin.y), color, window, cx);
            }
        },
    )
    .h(TICK_TEXT.line_height)
    .w_full()
}

/// The legend's bracket: the line and both ticks of an appointment's, 12 by 6 px.
pub(crate) fn bracket_glyph(theme: &Theme) -> impl IntoElement {
    let color = theme.text_muted;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.paint_quad(quad(
                bounds,
                Corners {
                    top_left: radius::XS,
                    top_right: radius::XS,
                    ..Corners::default()
                },
                transparent_black(),
                Edges {
                    top: size::APPOINTMENT_MARKER_STROKE,
                    left: size::APPOINTMENT_MARKER_STROKE,
                    right: size::APPOINTMENT_MARKER_STROKE,
                    bottom: px(0.),
                },
                color,
                BorderStyle::Solid,
            ))
        },
    )
    .w(px(12.))
    .h(px(6.))
    .flex_none()
}

pub(crate) fn swatch(kind: Option<SegmentKind>, theme: &Theme) -> impl IntoElement {
    let theme = theme.clone();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| match kind {
            Some(kind) => {
                for shape in segment_quad(kind, bounds, &theme) {
                    window.paint_quad(shape);
                }
            }
            None => window.paint_quad(quad(
                bounds,
                Corners::all(radius::XS),
                transparent_black(),
                px(1.),
                theme.text_placeholder,
                BorderStyle::Dashed,
            )),
        },
    )
    .size(size::ICON_XS)
    .flex_none()
}
