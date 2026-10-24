//! The WorkTime One theme and the other tokens of the WorkTime GPUI design system.
//!
//! `design/tokens.json` is a copy of the design system's tokens; the tests fail when a value
//! here drifts from it.

use std::sync::Arc;

use gpui::{
    App, BoxShadow, FontFeatures, FontWeight, Global, Hsla, Pixels, Styled, Window,
    WindowAppearance, hsla, point, px, rgba,
};

use crate::assets::{FONT_FAMILY, MONO_FONT_FAMILY};

/// Which variant windows use: the one Windows asks for, or one fixed by `--light` or `--dark`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl Global for ThemeMode {}

/// One layer of an elevation shadow, offset straight down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowLayer {
    pub y: f32,
    pub blur: f32,
    pub alpha: f32,
}

const fn layer(y: f32, blur: f32, alpha: f32) -> ShadowLayer {
    ShadowLayer { y, blur, alpha }
}

/// Colors and shadows of one WorkTime One variant. Each field is the design-system token of
/// the same name.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Hsla,
    pub surface: Hsla,
    pub elevated_surface: Hsla,
    pub editor: Hsla,
    pub border: Hsla,
    pub border_on_tint: Hsla,
    pub border_variant: Hsla,
    pub border_focused: Hsla,
    pub element: Hsla,
    pub element_hover: Hsla,
    pub element_active: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub accent: Hsla,
    pub accent_background: Hsla,
    pub accent_border: Hsla,
    pub success: Hsla,
    pub success_background: Hsla,
    pub success_border: Hsla,
    pub warning: Hsla,
    pub warning_background: Hsla,
    pub warning_border: Hsla,
    pub error: Hsla,
    pub error_background: Hsla,
    pub error_border: Hsla,
    pub hint: Hsla,
    pub tl_work: Hsla,
    pub tl_manual: Hsla,
    pub tl_pause: Hsla,
    pub tl_not_worked: Hsla,
    pub tl_gap_stripe: Hsla,
    pub tl_review: Hsla,
    pub tl_track: Hsla,
    pub tl_track_focused: Hsla,
    pub tl_now: Hsla,
    pub tl_on_fill: Hsla,
    /// The palette's matched text on its selected row, 4.5:1 on `element-active`.
    pub accent_on_active: Hsla,
    /// The palette's domain text on its selected row, 4.5:1 on `element-active`.
    pub text_muted_on_active: Hsla,
    /// The black veil behind a modal dialog or the palette.
    pub scrim: Hsla,
    /// The palette's shadow, which lifts it off the scrim.
    pub elevation_palette: [ShadowLayer; 4],
    pub elevation_elevated: [ShadowLayer; 2],
    pub elevation_modal: [ShadowLayer; 4],
}

fn color(hex: u32) -> Hsla {
    rgba(hex).into()
}

impl Theme {
    /// The variant for `window`: fixed by [`ThemeMode`], otherwise the window's appearance.
    pub fn current(window: &Window, cx: &App) -> Self {
        match cx.try_global::<ThemeMode>().copied().unwrap_or_default() {
            ThemeMode::System => Self::for_appearance(window.appearance()),
            ThemeMode::Light => Self::light(),
            ThemeMode::Dark => Self::dark(),
        }
    }

    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }

    pub fn dark() -> Self {
        let (accent, accent_background) = (color(0x74ade8ff), color(0x74ade81a));
        let (surface, editor, element) = (color(0x2f343eff), color(0x282c33ff), color(0x2e343eff));
        let (success, warning, text) = (color(0xa1c181ff), color(0xdec184ff), color(0xdce0e5ff));
        Self {
            background: color(0x3b414dff),
            surface,
            elevated_surface: color(0x2f343eff),
            editor,
            border: color(0x464b57ff),
            border_on_tint: color(0x5c6370ff),
            border_variant: color(0x363c46ff),
            border_focused: color(0x74ade8ff),
            element,
            element_hover: color(0x363c46ff),
            element_active: color(0x454a56ff),
            text,
            text_muted: color(0xa9afbcff),
            text_placeholder: color(0x878a98ff),
            accent,
            accent_background,
            accent_border: color(0x293b5bff),
            success,
            success_background: color(0xa1c1811a),
            success_border: color(0x38482fff),
            warning,
            warning_background: color(0xdec1841a),
            warning_border: color(0x5d4c2fff),
            error: color(0xdf8589ff),
            error_background: color(0xd072771a),
            error_border: color(0x4c2b2cff),
            hint: color(0x788ca6ff),
            tl_work: accent,
            tl_manual: success,
            tl_pause: color(0x4b5263ff),
            tl_not_worked: surface,
            tl_gap_stripe: warning,
            tl_review: accent_background,
            tl_track: element,
            tl_track_focused: editor,
            tl_now: text,
            tl_on_fill: color(0x282c33ff),
            accent_on_active: color(0x93bfedff),
            text_muted_on_active: color(0xb6bbc6ff),
            scrim: color(0x00000052),
            elevation_palette: [
                layer(2., 3., 0.12),
                layer(3., 6., 0.08),
                layer(6., 12., 0.04),
                layer(1., 0., 0.12),
            ],
            elevation_elevated: [layer(2., 3., 0.12), layer(1., 0., 0.06)],
            elevation_modal: [
                layer(2., 3., 0.12),
                layer(3., 6., 0.08),
                layer(6., 12., 0.04),
                layer(1., 0., 0.12),
            ],
        }
    }

    pub fn light() -> Self {
        let (accent, accent_background) = (color(0x4560c4ff), color(0xe2e2faff));
        let (surface, editor, element) = (color(0xebebecff), color(0xfafafaff), color(0xebebecff));
        let (success, warning, text) = (color(0x3f6e34ff), color(0x7d6400ff), color(0x242529ff));
        Self {
            background: color(0xdcdcddff),
            surface,
            elevated_surface: color(0xebebecff),
            editor,
            border: color(0xc9c9caff),
            border_on_tint: color(0xc9c9caff),
            border_variant: color(0xdfdfe0ff),
            border_focused: color(0x7d82e8ff),
            element,
            element_hover: color(0xdfdfe0ff),
            element_active: color(0xcacacaff),
            text,
            text_muted: color(0x58585aff),
            text_placeholder: color(0x7e8086ff),
            accent,
            accent_background,
            accent_border: color(0xcbcdf6ff),
            success,
            success_background: color(0xdfeadbff),
            success_border: color(0xc8dcc1ff),
            warning,
            warning_background: color(0xfaf2e6ff),
            warning_border: color(0xf4e7d1ff),
            error: color(0xad4234ff),
            error_background: color(0xfbdfd9ff),
            error_border: color(0xf6c6bdff),
            hint: color(0x7274a7ff),
            tl_work: accent,
            tl_manual: success,
            tl_pause: color(0xc9c9caff),
            tl_not_worked: surface,
            tl_gap_stripe: warning,
            tl_review: accent_background,
            tl_track: element,
            tl_track_focused: editor,
            tl_now: text,
            tl_on_fill: color(0xfafafaff),
            accent_on_active: color(0x354da6ff),
            text_muted_on_active: color(0x545456ff),
            scrim: color(0x0000002e),
            elevation_palette: [
                layer(2., 4., 0.12),
                layer(8., 16., 0.14),
                layer(16., 32., 0.12),
                layer(1., 0., 0.12),
            ],
            elevation_elevated: [layer(2., 3., 0.12), layer(1., 0., 0.03)],
            elevation_modal: [
                layer(2., 3., 0.06),
                layer(3., 6., 0.06),
                layer(6., 12., 0.04),
                layer(1., 0., 0.04),
            ],
        }
    }

    /// `elevation-elevated`: popover, menu, tooltip.
    pub fn shadow_elevated(&self) -> Vec<BoxShadow> {
        shadow(&self.elevation_elevated)
    }

    /// `elevation-modal`: dialog, flyout.
    pub fn shadow_modal(&self) -> Vec<BoxShadow> {
        shadow(&self.elevation_modal)
    }

    /// `elevation-palette`: the command palette, over a scrim.
    pub fn shadow_palette(&self) -> Vec<BoxShadow> {
        shadow(&self.elevation_palette)
    }
}

fn shadow(layers: &[ShadowLayer]) -> Vec<BoxShadow> {
    layers
        .iter()
        .map(|layer| BoxShadow {
            color: hsla(0., 0., 0., layer.alpha),
            offset: point(px(0.), px(layer.y)),
            blur_radius: px(layer.blur),
            spread_radius: px(0.),
            inset: false,
        })
        .collect()
}

/// Spacing at the normal density (`DynamicSpacing` in Zed).
pub mod spacing {
    use gpui::{Pixels, px};

    pub const BASE_02: Pixels = px(2.);
    pub const BASE_04: Pixels = px(4.);
    pub const BASE_06: Pixels = px(6.);
    pub const BASE_08: Pixels = px(8.);
    pub const BASE_12: Pixels = px(12.);
    pub const BASE_16: Pixels = px(16.);
    pub const BASE_20: Pixels = px(20.);
    pub const BASE_24: Pixels = px(24.);
    pub const BASE_32: Pixels = px(32.);
    pub const BASE_48: Pixels = px(48.);
}

pub mod motion;

pub mod radius {
    use gpui::{Pixels, px};

    pub const XS: Pixels = px(2.);
    pub const SM: Pixels = px(4.);
    pub const MD: Pixels = px(6.);
    pub const LG: Pixels = px(8.);
}

pub mod size {
    use gpui::{Pixels, px};

    pub const BUTTON_COMPACT: Pixels = px(18.);
    pub const BUTTON_DEFAULT: Pixels = px(22.);
    pub const BUTTON_MEDIUM: Pixels = px(28.);
    pub const BUTTON_LARGE: Pixels = px(32.);
    pub const ICON_XS: Pixels = px(12.);
    pub const ICON_SM: Pixels = px(14.);
    pub const ICON_MD: Pixels = px(16.);
    pub const TITLEBAR_HEIGHT: Pixels = px(32.);
    pub const STATUSBAR_HEIGHT: Pixels = px(28.);
    pub const FLYOUT_WIDTH: Pixels = px(360.);
    pub const TIMELINE_ROW: Pixels = px(16.);
    pub const TIMELINE_ROW_FOCUSED: Pixels = px(40.);
    /// Fits « Il manque 10 h 00 min » and « 10 h 00 min / 10 h 00 min » on one line.
    pub const DAY_COLUMN: Pixels = px(160.);

    // Not tokens of `tokens.json`: sizes the mockups and the week view spec give.
    /// The smallest key cap of a KeyBinding.
    pub const KEY_CAP: Pixels = px(20.);
    /// A window button of the TitleBar drawn by the app (Windows).
    pub const WINDOW_CONTROL_WIDTH: Pixels = px(46.);
    pub const POPOVER_WIDTH: Pixels = px(440.);
    /// A drag handle at each edge of the range selected on a timeline, and how far the range's
    /// outline overhangs the strip.
    pub const RANGE_HANDLE_WIDTH: Pixels = px(12.);
    pub const RANGE_HANDLE_HEIGHT: Pixels = px(30.);
    pub const RANGE_OVERHANG: Pixels = px(5.);
    /// The narrowest a range's outline shows.
    pub const RANGE_MIN_WIDTH: Pixels = px(16.);
    /// Where the pointer catches a range handle, wider than it shows.
    pub const RANGE_HANDLE_HIT: Pixels = px(24.);
    /// The command palette, narrower over a narrower window.
    pub const PALETTE_WIDTH: Pixels = px(520.);
    pub const WEEK_WINDOW_WIDTH: Pixels = px(980.);
    /// The narrowest week whose open card keeps its left column and the credit beside the
    /// corrections, as the French details need. Below [`WEEK_WINDOW_WIDTH`], so that the week
    /// opened at that width stays wide when a fractional scale reads its width back as 979.92.
    pub const WEEK_WIDE_MIN_WIDTH: Pixels = px(970.);
    /// The narrow open card still fits, below half of a screen 1536 wide (1920 at 125 %), where
    /// Windows Snap leaves 768.
    pub const WEEK_WINDOW_MIN_WIDTH: Pixels = px(760.);
    pub const WEEK_WINDOW_HEIGHT: Pixels = px(760.);
    /// The tallest open card, French details with two corrections, still fits.
    pub const WEEK_WINDOW_MIN_HEIGHT: Pixels = px(520.);
    /// A row of the « À régler » list.
    pub const LIST_ROW: Pixels = px(32.);
    /// The « maintenant » line and how far it overhangs the strip.
    pub const NOW_LINE_WIDTH: Pixels = px(2.);
    pub const NOW_LINE_OVERHANG: Pixels = px(3.);
    /// The bracket over a recurring appointment's planned hours: its stroke, the height of its
    /// ticks, and the room between it and the strip.
    pub const APPOINTMENT_MARKER_STROKE: Pixels = px(2.);
    pub const APPOINTMENT_MARKER_HEIGHT: Pixels = px(4.);
    pub const APPOINTMENT_MARKER_GAP: Pixels = px(7.);
    /// On the open day, 4 px clear of a selection's outline, its overhang and its 2 px halo.
    pub const APPOINTMENT_MARKER_GAP_FOCUSED: Pixels = px(11.);
    /// How tall the pointer finds the bracket, centred on it.
    pub const APPOINTMENT_MARKER_HIT: Pixels = px(14.);
    /// A bracket cut by now stops this far short of the « maintenant » line, after fading out
    /// over its last pixels.
    pub const APPOINTMENT_MARKER_NOW_CLEARANCE: Pixels = px(3.);
    pub const APPOINTMENT_MARKER_FADE: Pixels = px(10.);
    /// Between the bracket and its tooltip above it.
    pub const APPOINTMENT_TOOLTIP_GAP: Pixels = px(6.);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    /// IBM Plex Mono, for what the user edits and the timeline.
    Mono,
}

/// A text style of the type ramp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    pub family: FontFamily,
    pub size: Pixels,
    pub line_height: Pixels,
    pub weight: FontWeight,
    /// Tabular figures (`tnum`), so times and durations line up.
    pub tabular: bool,
}

const fn style(family: FontFamily, size: f32, line_height: f32, weight: f32) -> TypeStyle {
    TypeStyle {
        family,
        size: px(size),
        line_height: px(line_height),
        weight: FontWeight(weight),
        tabular: false,
    }
}

pub mod text {
    use super::FontFamily::{Mono, Sans};
    use super::{TypeStyle, style};

    pub const HEADLINE_LG: TypeStyle = style(Sans, 20., 28., 600.);
    pub const HEADLINE_MD: TypeStyle = style(Sans, 18., 26., 600.);
    pub const HEADLINE_SM: TypeStyle = style(Sans, 16., 24., 600.);
    pub const LABEL_LG: TypeStyle = style(Sans, 16., 24., 400.);
    pub const LABEL_MD: TypeStyle = style(Sans, 14., 20., 400.);
    pub const LABEL_STRONG: TypeStyle = style(Sans, 14., 20., 600.);
    pub const LABEL_SM: TypeStyle = style(Sans, 12., 16., 400.);
    pub const LABEL_XS: TypeStyle = style(Sans, 10., 14., 500.);
    pub const WT_HERO: TypeStyle = TypeStyle {
        tabular: true,
        ..style(Sans, 40., 48., 600.)
    };
    pub const WT_PRIMARY: TypeStyle = TypeStyle {
        tabular: true,
        ..style(Sans, 22., 28., 500.)
    };
    pub const WT_VALUE: TypeStyle = style(Mono, 14., 20., 400.);
    pub const WT_VALUE_SM: TypeStyle = style(Mono, 12., 16., 400.);
}

pub fn tabular_figures() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// Applies a [`TypeStyle`] to any styled element.
pub trait TypeStyled: Styled + Sized {
    fn type_style(self, style: TypeStyle) -> Self {
        let styled = match style.family {
            FontFamily::Sans => self.font_family(FONT_FAMILY),
            FontFamily::Mono => self.font_family(MONO_FONT_FAMILY),
        }
        .text_size(style.size)
        .line_height(style.line_height)
        .font_weight(style.weight);
        if style.tabular {
            styled.font_features(tabular_figures())
        } else {
            styled
        }
    }
}

impl<T: Styled> TypeStyled for T {}

#[cfg(test)]
mod tests;
