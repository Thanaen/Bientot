//! Checks every token against `design/tokens.json`, a copy of the design system's
//! `project/tokens.json` (https://claude.ai/artifact/BL74JiT44VFwkEk2wpCUnf, version
//! 1792292012-8732). After the design system changes, copy the new file over it: these tests
//! then name each token to update.

use std::collections::BTreeSet;

use serde_json::Value;

use super::*;

const TOKENS: &str = include_str!("../../design/tokens.json");

/// Tokens with no field here: the notification-area colors are baked into the ICO files.
const NOT_IN_THE_APP: [&str; 5] = [
    "tray-green",
    "tray-orange",
    "tray-red",
    "tray-blue",
    "tray-gray",
];

fn tokens() -> Value {
    serde_json::from_str(TOKENS).unwrap()
}

fn entries<'a>(tokens: &'a Value, family: &str) -> &'a [Value] {
    tokens[family]["tokens"].as_array().unwrap()
}

fn colors(theme: &Theme) -> Vec<(&'static str, Hsla)> {
    vec![
        ("background", theme.background),
        ("surface", theme.surface),
        ("elevated-surface", theme.elevated_surface),
        ("editor", theme.editor),
        ("border", theme.border),
        ("border-on-tint", theme.border_on_tint),
        ("border-variant", theme.border_variant),
        ("border-focused", theme.border_focused),
        ("element", theme.element),
        ("element-hover", theme.element_hover),
        ("element-active", theme.element_active),
        ("accent-on-active", theme.accent_on_active),
        ("text-muted-on-active", theme.text_muted_on_active),
        ("scrim", theme.scrim),
        ("text", theme.text),
        ("text-muted", theme.text_muted),
        ("text-placeholder", theme.text_placeholder),
        ("accent", theme.accent),
        ("accent-background", theme.accent_background),
        ("accent-border", theme.accent_border),
        ("success", theme.success),
        ("success-background", theme.success_background),
        ("success-border", theme.success_border),
        ("warning", theme.warning),
        ("warning-background", theme.warning_background),
        ("warning-border", theme.warning_border),
        ("error", theme.error),
        ("error-background", theme.error_background),
        ("error-border", theme.error_border),
        ("hint", theme.hint),
        ("tl-work", theme.tl_work),
        ("tl-manual", theme.tl_manual),
        ("tl-pause", theme.tl_pause),
        ("tl-not-worked", theme.tl_not_worked),
        ("tl-gap-stripe", theme.tl_gap_stripe),
        ("tl-review", theme.tl_review),
        ("tl-track", theme.tl_track),
        ("tl-track-focused", theme.tl_track_focused),
        ("tl-now", theme.tl_now),
        ("tl-on-fill", theme.tl_on_fill),
    ]
}

/// The hex value of a color token in one theme, following `{alias}` values.
fn hex(tokens: &Value, name: &str, theme: &str) -> String {
    let token = entries(tokens, "color")
        .iter()
        .find(|token| token["name"] == name)
        .unwrap_or_else(|| panic!("no color token {name}"));
    let value = token["value"][theme].as_str().unwrap();
    match value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        Some(alias) => hex(tokens, alias, theme),
        None => value.to_owned(),
    }
}

fn parse_hex(value: &str) -> Hsla {
    let digits = value.strip_prefix('#').unwrap();
    let rgba = match digits.len() {
        6 => u32::from_str_radix(digits, 16).unwrap() << 8 | 0xff,
        8 => u32::from_str_radix(digits, 16).unwrap(),
        _ => panic!("unexpected color {value}"),
    };
    color(rgba)
}

fn pixels(value: &Value) -> Pixels {
    let text = value.as_str().unwrap();
    px(text.strip_suffix("px").unwrap().parse().unwrap())
}

#[test]
fn every_color_token_matches_the_design_system_in_both_themes() {
    let tokens = tokens();
    for (id, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        for (name, value) in colors(&theme) {
            assert_eq!(
                value,
                parse_hex(&hex(&tokens, name, id)),
                "{name} in the {id} theme"
            );
        }
    }
}

#[test]
fn no_design_system_color_is_left_out() {
    let tokens = tokens();
    let in_file: BTreeSet<&str> = entries(&tokens, "color")
        .iter()
        .map(|token| token["name"].as_str().unwrap())
        .collect();
    let mut covered: BTreeSet<&str> = colors(&Theme::dark())
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    covered.extend(NOT_IN_THE_APP);
    assert_eq!(covered, in_file);
}

#[test]
fn lengths_match_the_design_system() {
    let tokens = tokens();
    let expected: Vec<(&str, &str, Pixels)> = vec![
        ("spacing", "base-02", spacing::BASE_02),
        ("spacing", "base-04", spacing::BASE_04),
        ("spacing", "base-06", spacing::BASE_06),
        ("spacing", "base-08", spacing::BASE_08),
        ("spacing", "base-12", spacing::BASE_12),
        ("spacing", "base-16", spacing::BASE_16),
        ("spacing", "base-20", spacing::BASE_20),
        ("spacing", "base-24", spacing::BASE_24),
        ("spacing", "base-32", spacing::BASE_32),
        ("spacing", "base-48", spacing::BASE_48),
        ("radius", "radius-xs", radius::XS),
        ("radius", "radius-sm", radius::SM),
        ("radius", "radius-md", radius::MD),
        ("radius", "radius-lg", radius::LG),
        ("size", "button-compact", size::BUTTON_COMPACT),
        ("size", "button-default", size::BUTTON_DEFAULT),
        ("size", "button-medium", size::BUTTON_MEDIUM),
        ("size", "button-large", size::BUTTON_LARGE),
        ("size", "icon-xs", size::ICON_XS),
        ("size", "icon-sm", size::ICON_SM),
        ("size", "icon-md", size::ICON_MD),
        ("size", "titlebar-height", size::TITLEBAR_HEIGHT),
        ("size", "statusbar-height", size::STATUSBAR_HEIGHT),
        ("size", "flyout-width", size::FLYOUT_WIDTH),
        ("size", "timeline-row", size::TIMELINE_ROW),
        ("size", "timeline-row-focused", size::TIMELINE_ROW_FOCUSED),
        ("size", "day-column", size::DAY_COLUMN),
    ];
    let mut in_file = BTreeSet::new();
    for family in ["spacing", "radius", "size"] {
        for token in entries(&tokens, family) {
            in_file.insert((family, token["name"].as_str().unwrap()));
            let (_, _, value) = expected
                .iter()
                .find(|(f, name, _)| *f == family && token["name"] == *name)
                .unwrap_or_else(|| panic!("no constant for {family} {}", token["name"]));
            assert_eq!(*value, pixels(&token["value"]), "{}", token["name"]);
        }
    }
    assert_eq!(in_file.len(), expected.len());
}

#[test]
fn text_styles_match_the_design_system() {
    let tokens = tokens();
    let expected = [
        ("headline-lg", text::HEADLINE_LG),
        ("headline-md", text::HEADLINE_MD),
        ("headline-sm", text::HEADLINE_SM),
        ("label-lg", text::LABEL_LG),
        ("label-md", text::LABEL_MD),
        ("label-strong", text::LABEL_STRONG),
        ("label-sm", text::LABEL_SM),
        ("label-xs", text::LABEL_XS),
        ("wt-hero", text::WT_HERO),
        ("wt-primary", text::WT_PRIMARY),
        ("wt-value", text::WT_VALUE),
        ("wt-value-sm", text::WT_VALUE_SM),
    ];
    let mut seen = 0;
    for group in tokens["type"]["groups"].as_array().unwrap() {
        for token in group["styles"].as_array().unwrap() {
            let name = token["name"].as_str().unwrap();
            let (_, style) = expected
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("no style for {name}"));
            let family = token.get("family").unwrap_or(&group["family"]);
            let family = match family.as_str().unwrap() {
                "sans" => FontFamily::Sans,
                "mono" => FontFamily::Mono,
                other => panic!("unknown family {other}"),
            };
            let tabular = token
                .get("fontFeatureSettings")
                .is_some_and(|features| features.as_str().unwrap().contains("\"tnum\" 1"));
            let from_file = TypeStyle {
                family,
                size: pixels(&token["fontSize"]),
                line_height: pixels(&token["lineHeight"]),
                weight: FontWeight(token["fontWeight"].as_f64().unwrap() as f32),
                tabular,
            };
            assert_eq!(*style, from_file, "{name}");
            seen += 1;
        }
    }
    assert_eq!(seen, expected.len());
}

/// Reads `0 2px 3px rgba(0,0,0,0.12), …`: black layers offset straight down, no spread.
fn parse_shadow(value: &str) -> Vec<ShadowLayer> {
    value
        .split("), ")
        .map(|layer| {
            let (lengths, color) = layer.split_once(" rgba(0,0,0,").unwrap();
            let lengths: Vec<f32> = lengths
                .split(' ')
                .map(|length| length.trim_end_matches("px").parse().unwrap())
                .collect();
            assert_eq!(lengths[0], 0., "horizontal offset in {value}");
            ShadowLayer {
                y: lengths[1],
                blur: lengths[2],
                alpha: color.trim_end_matches(')').parse().unwrap(),
            }
        })
        .collect()
}

#[test]
fn elevations_match_the_design_system() {
    let tokens = tokens();
    let shadows = entries(&tokens, "shadow");
    let value = |name: &str, theme: &str| {
        let token = shadows.iter().find(|token| token["name"] == name).unwrap();
        parse_shadow(token["value"][theme].as_str().unwrap())
    };
    for (id, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        assert_eq!(
            theme.elevation_elevated.to_vec(),
            value("elevation-elevated", id),
            "{id}"
        );
        assert_eq!(
            theme.elevation_modal.to_vec(),
            value("elevation-modal", id),
            "{id}"
        );
        assert_eq!(
            theme.elevation_palette.to_vec(),
            value("elevation-palette", id),
            "{id}"
        );
    }
    assert_eq!(shadows.len(), 3);
}

#[test]
fn the_palette_shadow_differs_from_the_modal_one_in_light() {
    assert_ne!(
        Theme::light().shadow_palette(),
        Theme::light().shadow_modal()
    );
}

#[test]
fn modal_shadow_is_three_soft_layers_and_a_contact_line() {
    let shadow = Theme::light().shadow_modal();
    assert_eq!(shadow.len(), 4);
    assert_eq!(shadow[3].offset, point(px(0.), px(1.)));
    assert_eq!(shadow[3].blur_radius, px(0.));
    assert_eq!(shadow[3].color, hsla(0., 0., 0., 0.04));
}

/// The WCAG contrast ratio of two opaque colors.
fn contrast(a: Hsla, b: Hsla) -> f32 {
    let luminance = |color: Hsla| {
        let rgb = color.to_rgb();
        let channel = |c: f32| {
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn the_palette_s_selected_row_reads_at_4_5_to_1() {
    for (id, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
        for (name, color) in [
            ("accent-on-active", theme.accent_on_active),
            ("text-muted-on-active", theme.text_muted_on_active),
            ("text", theme.text),
        ] {
            let ratio = contrast(color, theme.element_active);
            assert!(ratio >= 4.5, "{id} {name}: {ratio:.2}:1");
        }
    }
}
