//! What WorkTime's windows draw alike: the title bar over a transparent titlebar, the icons,
//! the disclosures, the small labels and capitals, the notices and the privacy footnote, the
//! thumb of a scroll bar and the shade of a scrolled edge, the tooltips, and a text shaped on
//! one line and how wide it is.

use gpui::{
    App, Bounds, ClickEvent, Div, ElementId, FocusHandle, Font, FontFeatures, Hsla, Pixels, Role,
    ScrollHandle, ShapedLine, SharedString, Stateful, TextRun, Window, WindowControlArea, canvas,
    div, fill, font, hsla, linear_color_stop, linear_gradient, point, prelude::*, px, size, svg,
    transparent_black,
};

use crate::assets::{FONT_FAMILY, MONO_FONT_FAMILY};
use crate::theme::spacing::*;
use crate::theme::{
    FontFamily, Theme, TypeStyle, TypeStyled as _, radius, size as token, tabular_figures, text,
};
use crate::week::WeekCommand;

/// What « Corriger le temps… », « Utiliser la détection intégrée » or « Exporter le diagnostic… »
/// left to show above the day, or what WorkTime tells unasked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub message: String,
    pub kind: NoticeKind,
    /// What « Annuler » runs, when the notice tells of what it can undo.
    pub undo: Option<WeekCommand>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Success,
    /// The day still needs a refresh: a correction was saved before one that failed, or a
    /// refresh failed.
    RefreshPending,
    /// A warning that asks for nothing the flyout can do.
    Warning,
    /// Invalid settings were set aside for the defaults: a warning with « Ouvrir les
    /// paramètres », to review them.
    SettingsRecovered,
    /// The built-in detection could not be saved: ActivityWatch is still the source.
    SourceSwitchFailed,
    Error,
}

impl Notice {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: NoticeKind::Success,
            undo: None,
        }
    }

    pub fn refresh_pending(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: NoticeKind::RefreshPending,
            undo: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: NoticeKind::Warning,
            undo: None,
        }
    }

    pub fn settings_recovered(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: NoticeKind::SettingsRecovered,
            undo: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: NoticeKind::Error,
            undo: None,
        }
    }

    pub fn source_switch_failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: NoticeKind::SourceSwitchFailed,
            undo: None,
        }
    }

    /// The notice with « Annuler », which runs `undo`.
    pub fn with_undo(self, undo: Option<WeekCommand>) -> Self {
        Self { undo, ..self }
    }

    /// Whether the notice still holds once the day is shown again. The warning asks for a
    /// refresh, so a refresh that recalculated the day ends it; a success or a failed switch
    /// stays until the flyout closes, through the periodic refreshes.
    pub(crate) fn outlives(&self, recalculated: bool) -> bool {
        !(self.kind == NoticeKind::RefreshPending && recalculated)
    }
}

/// `title`, which drags the window, then `controls`, then the close button, which runs
/// `on_close`.
pub(crate) fn title_bar(
    title: impl IntoElement,
    controls: impl IntoIterator<Item = Stateful<Div>>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    theme: &Theme,
) -> Div {
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
                .child(small(title, theme.text_muted)),
        )
        .children(controls)
        .child(control("close", "x", WindowControlArea::Close, theme).on_click(on_close))
}

/// A button of the title bar, drawn as `icon`, that Windows treats as `area`.
pub(crate) fn control(
    id: &'static str,
    icon: &str,
    area: WindowControlArea,
    theme: &Theme,
) -> Stateful<Div> {
    let hover = theme.element_hover;
    div()
        .id(id)
        .window_control_area(area)
        .w(token::WINDOW_CONTROL_WIDTH)
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(hover))
        .child(self::icon(icon, theme.text_muted).size(token::ICON_XS))
}

pub(crate) fn icon(name: &str, color: Hsla) -> gpui::Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(token::ICON_MD)
        .flex_none()
        .text_color(color)
}

/// The design system's info, success, warning and error, which a [`callout_box`] or a bar
/// is tinted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hue {
    Info,
    Success,
    Warning,
    Error,
}

/// The colours of a [`Hue`]: the box's fill and border, and its icon's.
pub(crate) struct HueColors {
    pub(crate) background: Hsla,
    pub(crate) border: Hsla,
    pub(crate) foreground: Hsla,
}

impl Hue {
    pub(crate) fn colors(self, theme: &Theme) -> HueColors {
        let (background, border, foreground) = match self {
            Hue::Info => (theme.accent_background, theme.accent_border, theme.accent),
            Hue::Success => (
                theme.success_background,
                theme.success_border,
                theme.success,
            ),
            Hue::Warning => (
                theme.warning_background,
                theme.warning_border,
                theme.warning,
            ),
            Hue::Error => (theme.error_background, theme.error_border, theme.error),
        };
        HueColors {
            background,
            border,
            foreground,
        }
    }

    /// The icon of a callout of this hue, unless it says something more precise.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Hue::Info => "info",
            Hue::Success => "circle-check",
            Hue::Warning => "triangle-alert",
            Hue::Error => "circle-alert",
        }
    }
}

/// The design system's Callout: the icon of `hue` on the first line of what the caller adds
/// after it, in a box of that hue.
pub(crate) fn callout_box(hue: Hue, theme: &Theme) -> Div {
    callout_box_with_icon(hue, hue.icon(), theme)
}

/// A [`callout_box`] that shows `icon_name` instead of its hue's icon.
pub(crate) fn callout_box_with_icon(hue: Hue, icon_name: &str, theme: &Theme) -> Div {
    lowered_callout_box(hue, icon_name, px(0.), theme)
}

/// A [`callout_box_with_icon`] whose first line is `lowered` below the top of what the caller
/// adds, the icon with it.
fn lowered_callout_box(hue: Hue, icon_name: &str, lowered: Pixels, theme: &Theme) -> Div {
    let foreground = hue.colors(theme).foreground;
    callout_frame(hue, theme)
        .items_start()
        .child(icon(icon_name, foreground).mt(BASE_02 + lowered))
}

/// How far a line of [`text::LABEL_MD`] is lowered to centre on a [`token::BUTTON_MEDIUM`]
/// button beside it.
pub(crate) fn beside_button_inset() -> Pixels {
    (token::BUTTON_MEDIUM - text::LABEL_MD.line_height) / 2.
}

/// The design system's inline Callout, for one line and its action: the icon of `hue`
/// centred on the row the caller adds after it, in a box of that hue.
pub(crate) fn inline_callout_box(hue: Hue, theme: &Theme) -> Div {
    let foreground = hue.colors(theme).foreground;
    callout_frame(hue, theme)
        .items_center()
        .child(icon(hue.icon(), foreground))
}

/// The box of a Callout of `hue`, with nothing in it yet.
fn callout_frame(hue: Hue, theme: &Theme) -> Div {
    let HueColors {
        background, border, ..
    } = hue.colors(theme);
    div()
        .flex()
        .gap(BASE_08)
        .py(BASE_08)
        .px(BASE_12)
        .rounded(radius::MD)
        .border_1()
        .border_color(border)
        .bg(background)
}

/// The frame of `notice`, its kind's icon first, in its kind's colours; the caller adds the
/// message.
pub(crate) fn notice_box(
    id: impl Into<ElementId>,
    notice: &Notice,
    theme: &Theme,
) -> Stateful<Div> {
    lowered_notice_box(id, notice, px(0.), theme)
}

/// A [`notice_box`] whose message shares its first line with a [`token::BUTTON_MEDIUM`]
/// button: the icon is lowered by [`beside_button_inset`], as the caller lowers the message.
pub(crate) fn notice_box_beside_button(
    id: impl Into<ElementId>,
    notice: &Notice,
    theme: &Theme,
) -> Stateful<Div> {
    lowered_notice_box(id, notice, beside_button_inset(), theme)
}

fn lowered_notice_box(
    id: impl Into<ElementId>,
    notice: &Notice,
    lowered: Pixels,
    theme: &Theme,
) -> Stateful<Div> {
    let (hue, role) = match notice.kind {
        NoticeKind::Success => (Hue::Success, Role::Status),
        NoticeKind::RefreshPending | NoticeKind::Warning | NoticeKind::SettingsRecovered => {
            (Hue::Warning, Role::Status)
        }
        NoticeKind::SourceSwitchFailed | NoticeKind::Error => (Hue::Error, Role::Alert),
    };
    lowered_callout_box(hue, hue.icon(), lowered, theme)
        .id(id)
        .role(role)
        .aria_label(SharedString::from(notice.message.clone()))
}

/// A link that shows or hides what follows it, its chevron down while `open`; the caller makes
/// it toggle.
pub(crate) fn disclosure(
    id: impl Into<ElementId>,
    label: &'static str,
    open: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let (accent, focused) = (theme.accent, theme.border_focused);
    let chevron = if open {
        "chevron-down"
    } else {
        "chevron-right"
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .aria_expanded(open)
        .tab_index(0)
        .h(token::BUTTON_DEFAULT)
        .flex()
        .flex_none()
        .items_center()
        .gap(BASE_04)
        .type_style(text::LABEL_SM)
        .rounded(radius::SM)
        .border_1()
        .border_color(transparent_black())
        .whitespace_nowrap()
        .text_color(accent)
        .cursor_pointer()
        .hover(|style| style.underline())
        .focus_visible(move |style| style.border_color(focused))
        .child(icon(chevron, accent).size(token::ICON_XS))
        .child(label)
}

/// The small capitals over a list: « CORRECTIONS MANUELLES ».
pub(crate) fn caps(label: &str, theme: &Theme) -> Div {
    div()
        .type_style(text::LABEL_XS)
        .text_color(theme.text_muted)
        .child(label.to_uppercase())
}

/// The design system's Tooltip: short lines on an elevated surface, each on one line so that a
/// time range is never wrapped at its dash. The caller adds the lines as children.
pub(crate) fn tooltip_box(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .whitespace_nowrap()
        .px(BASE_08)
        .py(BASE_04)
        .rounded(radius::LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.elevated_surface)
        .shadow(theme.shadow_elevated())
        .type_style(text::LABEL_SM)
        .font_features(tabular_figures())
        .text_color(theme.text)
}

/// How wide `text` is on one line in `style`.
pub(crate) fn text_width(text: &str, style: TypeStyle, window: &Window) -> Pixels {
    shape_text(text, style, transparent_black(), window).width
}

/// `text` laid out on one line in `style` and `color`, to measure or paint by hand.
pub(crate) fn shape_text(text: &str, style: TypeStyle, color: Hsla, window: &Window) -> ShapedLine {
    let family = match style.family {
        FontFamily::Sans => FONT_FAMILY,
        FontFamily::Mono => MONO_FONT_FAMILY,
    };
    let features = if style.tabular {
        tabular_figures()
    } else {
        FontFeatures::default()
    };
    let run = TextRun {
        len: text.len(),
        font: Font {
            weight: style.weight,
            features,
            ..font(family)
        },
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        style.size,
        &[run],
        None,
    )
}

pub(crate) fn small(child: impl IntoElement, color: Hsla) -> impl IntoElement {
    div()
        .type_style(text::LABEL_SM)
        .text_color(color)
        .child(child)
}

/// A line at the foot of a page, after a shield: what stays private.
pub(crate) fn footnote(label: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(BASE_06)
        .type_style(text::LABEL_SM)
        .text_color(theme.text_muted)
        .child(icon("shield", theme.text_muted).size(token::ICON_SM))
        .child(label)
}

/// The thumb of the scroll bar of what `scroll` tracks, while it overflows, drawn over its right
/// edge in the `relative` parent. It reads the handle when it paints, once the content laid out
/// in this frame.
pub(crate) fn scroll_thumb(scroll: ScrollHandle, theme: &Theme) -> impl IntoElement {
    let color = theme.text_placeholder;
    canvas(
        |_, _, _| {},
        move |track, _, window, _| {
            let overflow = scroll.max_offset().y;
            if overflow <= px(0.) {
                return;
            }
            let viewport = scroll.bounds().size.height;
            let height = (track.size.height * (viewport / (viewport + overflow))).max(BASE_16);
            let top = (track.size.height - height) * (-scroll.offset().y / overflow);
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(track.origin.x, track.origin.y + top),
                        size(track.size.width, height),
                    ),
                    color,
                )
                .corner_radii(track.size.width / 2.),
            );
        },
    )
    .absolute()
    .top(BASE_04)
    .bottom(BASE_04)
    .right(BASE_04)
    .w(px(4.))
}

/// A shadow over the top edge of what `scroll` tracks while it is scrolled down, in the
/// `relative` parent, so that what scrolled out of view leaves a cue. It reads the offset when it
/// paints, after the content applied a scroll asked in the same frame.
pub(crate) fn scrolled_edge(scroll: ScrollHandle, theme: &Theme) -> impl IntoElement {
    let layer = theme.elevation_elevated[0];
    let shade = hsla(0., 0., 0., layer.alpha);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if is_scrolled_down(&scroll) {
                window.paint_quad(fill(
                    bounds,
                    linear_gradient(
                        180.,
                        linear_color_stop(shade, 0.),
                        linear_color_stop(shade.opacity(0.), 1.),
                    ),
                ));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .h(px(layer.y + layer.blur))
}

/// Whether what `scroll` tracks is scrolled down. GPUI orders pixels by `total_cmp`, under which
/// the -0 its clamp leaves on content that does not overflow is below 0.
fn is_scrolled_down(scroll: &ScrollHandle) -> bool {
    f32::from(scroll.offset().y) < 0.
}

/// Gives `handle` the focus on a press of the mouse that no control under it took (a focusable
/// element takes the focus on its press and prevents the default), and keeps it from the
/// focusable elements around.
pub(crate) fn focus_unless_taken(handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    if !window.default_prevented() {
        window.focus(handle, cx);
        window.prevent_default();
    }
}

/// A handle for an element to `track_focus` that Tab reaches. GPUI applies an element's
/// `tab_index` only to the handle it makes itself, never to one the element tracks.
pub(crate) fn tab_stop(cx: &App) -> FocusHandle {
    cx.focus_handle().tab_stop(true)
}

/// The two parts of a time field.
pub(crate) struct ClockFocus {
    pub hours: FocusHandle,
    pub minutes: FocusHandle,
}

impl ClockFocus {
    pub fn new(cx: &App) -> Self {
        Self {
            hours: tab_stop(cx),
            minutes: tab_stop(cx),
        }
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.hours.is_focused(window) || self.minutes.is_focused(window)
    }
}

#[cfg(all(test, feature = "screenshots", target_os = "linux"))]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui::{AppContext as _, Bounds, Context, HeadlessAppContext, Render};

    use super::*;
    use crate::assets::{self, Assets};
    use crate::strings::Language;
    use crate::theme::ThemeMode;

    /// A wheel turned down on days that fit leaves GPUI's offset at -0: no shadow (#306).
    #[test]
    fn content_that_fits_is_not_scrolled_down() {
        let scroll = ScrollHandle::new();
        for (offset, scrolled) in [(px(-0.), false), (px(0.), false), (px(-1.), true)] {
            scroll.set_offset(point(px(0.), offset));
            assert_eq!(is_scrolled_down(&scroll), scrolled, "{offset:?}");
        }
    }

    const ONE_LINE: &str = "Horaires proposés";
    const LONG: &str = "Temps non renseigné · 25 min 30 s. Travailliez-vous à ce moment-là ? \
                        Indiquez les horaires travaillés.";

    /// A callout of `body` in `frame` that keeps where its icon and its body were laid out.
    struct Callout {
        frame: fn(Hue, &Theme) -> Div,
        body: &'static str,
        laid_out: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    }

    impl Render for Callout {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = Theme::current(window, cx);
            let laid_out = self.laid_out.clone();
            div().w(px(240.)).type_style(text::LABEL_MD).child(
                (self.frame)(Hue::Info, &theme)
                    .on_children_prepainted(move |bounds, _, _| *laid_out.borrow_mut() = bounds)
                    .child(div().flex_1().min_w_0().child(self.body)),
            )
        }
    }

    /// Where the icon and the body of a callout of `body` in `frame` are laid out.
    fn layout(
        frame: fn(Hue, &Theme) -> Div,
        body: &'static str,
    ) -> (Bounds<Pixels>, Bounds<Pixels>) {
        let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
            assets::FONT_FAMILY,
        ));
        let mut cx = HeadlessAppContext::with_asset_source(text_system, Arc::new(Assets));
        cx.update(|cx| {
            cx.set_global(ThemeMode::Light);
            cx.set_global(Language::French);
            assets::load_fonts(cx).unwrap();
        });
        let laid_out = Rc::default();
        let view = Callout {
            frame,
            body,
            laid_out: Rc::clone(&laid_out),
        };
        let window = cx
            .open_window(size(px(400.), px(300.)), |_, cx| cx.new(|_| view))
            .unwrap();
        cx.update_window(window.into(), |_, window, _| window.refresh())
            .unwrap();
        cx.run_until_parked();
        let laid_out = laid_out.borrow();
        let [icon, body] = laid_out[..] else {
            panic!("a callout lays out its icon and its body: {laid_out:?}");
        };
        (icon, body)
    }

    /// As the design system's Callout: the icon sits on the first line of the body however
    /// many lines follow, 8 px before it.
    #[test]
    fn the_icon_is_centred_on_the_first_line_of_the_body() {
        for body in [ONE_LINE, LONG] {
            let (icon, text_box) = layout(callout_box, body);
            let first_line = text_box.top() + text::LABEL_MD.line_height / 2.;
            assert_eq!(icon.center().y, first_line, "{body}");
            assert_eq!(text_box.left() - icon.right(), BASE_08, "{body}");
        }
        assert!(
            layout(callout_box, LONG).1.size.height > text::LABEL_MD.line_height,
            "the long body wraps"
        );
    }

    /// As the design system's inline Callout: the icon sits in the middle of the row however
    /// many lines the body takes, 8 px before it.
    #[test]
    fn the_inline_icon_is_centred_on_the_row() {
        for body in [ONE_LINE, LONG] {
            let (icon, text_box) = layout(inline_callout_box, body);
            assert_eq!(icon.center().y, text_box.center().y, "{body}");
            assert_eq!(text_box.left() - icon.right(), BASE_08, "{body}");
        }
    }
}

/// A control that cannot run now: out of the Tab order and deaf to clicks. It keeps its id, so
/// that a control pressed with the keyboard keeps the focus once it disables itself.
pub fn disabled_control(
    id: &'static str,
    name: SharedString,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let focused = theme.border_focused;
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(name)
        // GPUI has no setter for this state, only this hook on the node.
        .a11y_synthetic_children(|children| children.parent_node().set_disabled())
        .tab_index(0)
        .tab_stop(false)
        .rounded(radius::SM)
        .border_1()
        .border_color(gpui::transparent_black())
        .text_color(theme.text_placeholder)
        .focus_visible(move |style| style.border_color(focused))
        // Without this, GPUI focuses a clicked node that has a tab index.
        .on_any_mouse_down(|_, window, _| window.prevent_default())
}
