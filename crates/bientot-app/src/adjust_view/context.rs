//! The unrecorded time a declaration opened on, in its day: the day's strip with Début–Fin
//! marked on it, a choice of slot when there are several, and what came before and after.

use gpui::{
    BorderStyle, Bounds, Context, Corners, FontWeight, Role, SharedString, Toggled, canvas, div,
    fill, point, prelude::*, px, quad, size, transparent_black,
};

use super::{AdjustView, label};
use crate::adjust::{AroundRow, DayContext};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, tabular_figures, text};
use crate::week;
use crate::week_view::{segment_quad, swatch};

/// The strip's height: a week row's, so that the hatching reads the same.
const STRIP_HEIGHT: gpui::Pixels = token::TIMELINE_ROW;
/// The rows' times, wide enough for « 09:40:12–09:45:42 » in tabular figures, medium weight.
const TIMES_COLUMN: gpui::Pixels = px(136.);
/// The outline of Début–Fin around the strip.
const MARK_WIDTH: gpui::Pixels = px(2.);

impl AdjustView {
    pub(super) fn day_context(
        &self,
        context: &DayContext,
        locked: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("adjust-context")
            .flex()
            .flex_col()
            .gap(BASE_08)
            .child(label(context.title, theme))
            .child(strip(context, theme))
            .when(!context.slots.is_empty(), |section| {
                section.child(self.slots(context, locked, theme, cx))
            })
            .child(around(&context.around, theme))
    }

    fn slots(
        &self,
        context: &DayContext,
        locked: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let choices = context.slots.iter().enumerate().map(|(index, slot)| {
            let selected = slot.selected;
            let (background, border, color) = match (locked, selected) {
                (false, true) => (theme.accent_background, theme.accent, theme.accent),
                (true, true) => (theme.element_active, theme.border, theme.text_placeholder),
                (false, false) => (theme.editor, theme.border, theme.text),
                (true, false) => (theme.element, theme.border_variant, theme.text_placeholder),
            };
            let focused = theme.border_focused;
            let hover = theme.element_hover;
            let choice = div()
                .id(("slot", index))
                .role(Role::RadioButton)
                .aria_label(SharedString::from(slot.name.clone()))
                .aria_toggled(if selected {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .h(token::BUTTON_DEFAULT)
                .px(BASE_08)
                .flex()
                .items_center()
                .rounded(radius::SM)
                .border_1()
                .border_color(border)
                .bg(background)
                .text_color(color)
                .type_style(text::LABEL_SM)
                .font_features(tabular_figures())
                .when(selected, |choice| choice.font_weight(FontWeight::MEDIUM))
                .whitespace_nowrap()
                .child(slot.times.clone());
            if locked {
                return choice;
            }
            choice
                .tab_index(0)
                .cursor_pointer()
                .when(!selected, |choice| {
                    choice.hover(move |style| style.bg(hover))
                })
                .focus_visible(move |style| style.border_color(focused))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.change(cx, |form| form.choose_slot(index))
                }))
        });
        div()
            .id("slots")
            .role(Role::RadioGroup)
            .aria_label(context.slots_name)
            .flex()
            .flex_wrap()
            .gap(BASE_06)
            .children(choices)
    }
}

/// The day from arrival, Début–Fin outlined on it, its two ends written under it.
fn strip(context: &DayContext, theme: &Theme) -> impl IntoElement {
    let pieces = context.strip.clone();
    let marked = context.marked;
    let painted = theme.clone();
    let bar = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let theme = &painted;
            window.paint_quad(fill(bounds, theme.tl_track).corner_radii(Corners::all(radius::XS)));
            let width = f32::from(bounds.size.width);
            for piece in &pieces {
                let (x, w) = week::place(piece.start, piece.length, width);
                let placed = Bounds::new(
                    point(bounds.origin.x + px(x), bounds.origin.y),
                    size(px(w), bounds.size.height),
                );
                for shape in segment_quad(piece.kind, placed, theme) {
                    window.paint_quad(shape);
                }
            }
            if let Some((start, end)) = marked {
                let (x, w) = week::place(start, end - start, width);
                let outline = Bounds::new(
                    point(
                        bounds.origin.x + px(x) - MARK_WIDTH,
                        bounds.origin.y - MARK_WIDTH,
                    ),
                    size(
                        px(w) + MARK_WIDTH * 2.,
                        bounds.size.height + MARK_WIDTH * 2.,
                    ),
                );
                window.paint_quad(quad(
                    outline,
                    Corners::all(radius::SM),
                    transparent_black(),
                    MARK_WIDTH,
                    theme.accent,
                    BorderStyle::Solid,
                ));
                // A ring of the window's background inside the outline, which would melt into
                // the work segments of the same accent otherwise.
                let ring = Bounds::new(
                    outline.origin + point(MARK_WIDTH, MARK_WIDTH),
                    outline.size - size(MARK_WIDTH * 2., MARK_WIDTH * 2.),
                );
                window.paint_quad(quad(
                    ring,
                    Corners::all(radius::XS),
                    transparent_black(),
                    px(1.),
                    theme.editor,
                    BorderStyle::Solid,
                ));
            }
        },
    )
    .w_full()
    .h(STRIP_HEIGHT);
    div()
        .flex()
        .flex_col()
        .gap(BASE_04)
        // Room for the outline, which overhangs the strip.
        .pt(MARK_WIDTH)
        .px(MARK_WIDTH)
        .child(bar)
        .child(
            div()
                .flex()
                .justify_between()
                .type_style(text::LABEL_SM)
                .font_features(tabular_figures())
                .text_color(theme.text_muted)
                .child(context.from.clone())
                .child(context.to.clone()),
        )
}

/// What came before the chosen slot, the slot, and what came after, one row each.
fn around(rows: &[AroundRow], theme: &Theme) -> impl IntoElement {
    div()
        .id("around")
        .role(Role::List)
        .flex()
        .flex_col()
        .gap(BASE_02)
        .children(rows.iter().enumerate().map(|(index, row)| {
            div()
                .id(("around", index))
                .role(Role::ListItem)
                .aria_label(SharedString::from(row.name.clone()))
                .flex()
                .items_center()
                .gap(BASE_08)
                .px(BASE_08)
                .py(BASE_02)
                .rounded(radius::SM)
                .when(row.chosen, |line| {
                    line.bg(theme.element)
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!row.chosen, |line| line.text_color(theme.text_muted))
                .child(swatch(Some(row.kind), theme))
                .child(
                    div()
                        .w(TIMES_COLUMN)
                        .flex_none()
                        .font_features(tabular_figures())
                        .whitespace_nowrap()
                        .child(row.times.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(row.label),
                )
                .child(
                    div()
                        .flex_none()
                        .font_features(tabular_figures())
                        .whitespace_nowrap()
                        .child(row.duration.clone()),
                )
        }))
}
