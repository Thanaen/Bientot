//! The lines of a plugin's card, drawn by the app from the prepared view tree (contract
//! `view-tree.md`, FR-041): the app's text styles, colours and duration formats, never the
//! plugin's. Durations, countdowns and times read the clock at each drawing, so they advance
//! without waking the plugin.

use bientot_plugin_api::view::{Prepared, Role as TextRole, Tone};
use gpui::{AnyElement, FontWeight, Role, SharedString, div, prelude::*, px, relative};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::strings::Language;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, tabular_figures, text};
use crate::time_text;

/// What `node` reads at `now`: its text, its time, or the label of its progress bar. A stack or
/// a row reads its children, one after the other.
pub fn spoken(node: &Prepared, now: Timestamp, zone: &TimeZone, language: Language) -> String {
    match node {
        Prepared::Stack(children) | Prepared::Row(children) => children
            .iter()
            .map(|child| spoken(child, now, zone, language))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Prepared::Progress { label, .. } => label.clone(),
        node => shown(node, now, zone, language).unwrap_or_default(),
    }
}

/// The text of a leaf at `now`: `None` for a stack, a row or a progress bar.
fn shown(node: &Prepared, now: Timestamp, zone: &TimeZone, language: Language) -> Option<String> {
    Some(match node {
        Prepared::Text { text, .. } | Prepared::Badge { text, .. } => text.clone(),
        // « 1 h 30 min », to the nearest minute like the counted time, growing from `since`.
        Prepared::Duration { value, since } => {
            let grown = since.map_or(SignedDuration::ZERO, |since| {
                now.duration_since(since).max(SignedDuration::ZERO)
            });
            time_text::minutes_text(SignedDuration::from_secs(*value) + grown, language)
        }
        // The minutes left, rounded up: « 1 min » until the end, never « 0 min » before it.
        Prepared::Countdown { until } => {
            let left = until.duration_since(now).max(SignedDuration::ZERO);
            let seconds = left.as_secs();
            let minutes = seconds / 60 + i64::from(seconds % 60 > 0 || left.subsec_nanos() > 0);
            time_text::minutes_text(SignedDuration::from_mins(minutes), language)
        }
        Prepared::Time { at } => {
            let local = at.to_zoned(zone.clone()).datetime();
            language.short_time(local.round(jiff::Unit::Minute).unwrap_or(local).time())
        }
        Prepared::Stack(_) | Prepared::Row(_) | Prepared::Progress { .. } => return None,
    })
}

/// What draws the lines of one card: the clock and zone of the drawing, and a count of its
/// progress bars, which need an id of their own.
pub struct Drawing<'a> {
    pub now: Timestamp,
    pub zone: &'a TimeZone,
    pub language: Language,
    pub theme: &'a Theme,
    bars: usize,
}

impl<'a> Drawing<'a> {
    pub fn new(now: Timestamp, zone: &'a TimeZone, language: Language, theme: &'a Theme) -> Self {
        Self {
            now,
            zone,
            language,
            theme,
            bars: 0,
        }
    }

    /// One line of a card: at most one line high, what does not fit ends with an ellipsis.
    pub fn line(&mut self, node: &Prepared) -> AnyElement {
        self.element(node)
    }

    fn element(&mut self, node: &Prepared) -> AnyElement {
        let (now, zone, language, theme) = (self.now, self.zone, self.language, self.theme);
        match node {
            Prepared::Stack(children) => div()
                .flex()
                .flex_col()
                .gap(BASE_02)
                .min_w_0()
                .children(
                    children
                        .iter()
                        .map(|child| self.element(child))
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
            // The first child takes the room and is cut, the others keep their width at the end:
            // a label, then its value on the right, as the flyout's own lines.
            Prepared::Row(children) => div()
                .flex()
                .items_center()
                .gap(BASE_08)
                .min_w_0()
                .children(
                    children
                        .iter()
                        .enumerate()
                        .map(|(index, child)| {
                            let child = self.element(child);
                            if index == 0 {
                                div().flex_1().min_w_0().child(child).into_any_element()
                            } else {
                                div().flex_none().child(child).into_any_element()
                            }
                        })
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
            Prepared::Text { text, role } => styled_text(*role, theme)
                .truncate()
                .child(text.clone())
                .into_any_element(),
            Prepared::Badge { text, tone } => badge(text.clone(), *tone, theme),
            Prepared::Progress { value, max, label } => {
                let share = if *max > 0. {
                    (value / max).clamp(0., 1.) as f32
                } else {
                    0.
                };
                self.bars += 1;
                div()
                    .id(("plugin-progress", self.bars))
                    .role(Role::ProgressIndicator)
                    .aria_label(SharedString::from(label.clone()))
                    .aria_numeric_value(f64::from(share * 100.))
                    .w_full()
                    .h(px(4.))
                    .my(BASE_08)
                    .rounded(px(2.))
                    .bg(theme.element_active)
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(relative(share))
                            .rounded(px(2.))
                            // The accent is the counted time's: a plugin's bar stays neutral.
                            .bg(theme.text_muted),
                    )
                    .into_any_element()
            }
            leaf => div()
                .font_features(tabular_figures())
                .whitespace_nowrap()
                .child(shown(leaf, now, zone, language).unwrap_or_default())
                .into_any_element(),
        }
    }
}

fn styled_text(role: TextRole, theme: &Theme) -> gpui::Div {
    match role {
        TextRole::Title => div().type_style(text::LABEL_STRONG),
        TextRole::Muted => div().text_color(theme.text_muted),
        TextRole::Value => div()
            .font_features(tabular_figures())
            .font_weight(FontWeight::MEDIUM),
        TextRole::Mono => div().type_style(text::WT_VALUE),
        TextRole::Body | TextRole::Unknown => div(),
    }
}

/// The design system's badge, as the week's « N à revoir »: a tinted pill of small text.
fn badge(label: String, tone: Tone, theme: &Theme) -> AnyElement {
    let (background, border, foreground) = match tone {
        Tone::Info => (theme.accent_background, theme.accent_border, theme.accent),
        Tone::Success => (
            theme.success_background,
            theme.success_border,
            theme.success,
        ),
        Tone::Warning => (
            theme.warning_background,
            theme.warning_border,
            theme.warning,
        ),
        Tone::Error => (theme.error_background, theme.error_border, theme.error),
        Tone::Muted | Tone::Unknown => (theme.element, theme.border, theme.text_muted),
    };
    div()
        .flex()
        .child(
            div()
                .h(px(18.))
                .px(BASE_06)
                .flex()
                .items_center()
                .rounded(radius::MD)
                .border_1()
                .border_color(border)
                .bg(background)
                .text_color(foreground)
                .type_style(text::LABEL_SM)
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(label),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn durations_countdowns_and_times_read_the_clock_of_the_drawing() {
        let zone = TimeZone::get("Europe/Paris").unwrap();
        let now = at("2026-11-09T14:00:00Z");
        let read = |node: &Prepared, language| shown(node, now, &zone, language).unwrap();
        let duration = |value, since: Option<&str>| Prepared::Duration {
            value,
            since: since.map(at),
        };
        assert_eq!(
            read(&duration(5400, None), Language::French),
            "1\u{a0}h\u{a0}30\u{a0}min"
        );
        assert_eq!(read(&duration(1500, None), Language::English), "25m");
        assert_eq!(
            read(
                &duration(0, Some("2026-11-09T13:18:00Z")),
                Language::English
            ),
            "42m",
            "it grows from `since`"
        );
        assert_eq!(
            read(
                &duration(60, Some("2026-11-09T15:00:00Z")),
                Language::English
            ),
            "1m",
            "a `since` to come adds nothing"
        );
        let countdown = |until| Prepared::Countdown { until: at(until) };
        assert_eq!(
            read(&countdown("2026-11-09T14:25:00Z"), Language::English),
            "25m"
        );
        assert_eq!(
            read(&countdown("2026-11-09T14:24:01Z"), Language::English),
            "25m"
        );
        assert_eq!(
            read(&countdown("2026-11-09T14:00:20Z"), Language::English),
            "1m"
        );
        assert_eq!(
            read(&countdown("2026-11-09T13:00:00Z"), Language::English),
            "0m"
        );
        let time = Prepared::Time {
            at: at("2026-11-09T15:55:20Z"),
        };
        assert_eq!(read(&time, Language::French), "16:55", "in the user's zone");
        assert_eq!(
            read(&time, Language::English),
            "4:55\u{a0}PM",
            "as the app writes times"
        );
    }

    #[test]
    fn a_line_reads_its_texts_in_order_and_a_bar_its_label() {
        let zone = TimeZone::UTC;
        let now = at("2026-11-09T14:00:00Z");
        let row = Prepared::Row(vec![
            Prepared::Text {
                text: "Longest focus".into(),
                role: TextRole::Body,
            },
            Prepared::Duration {
                value: 5400,
                since: None,
            },
        ]);
        assert_eq!(
            spoken(&row, now, &zone, Language::English),
            "Longest focus 1h\u{a0}30m"
        );
        let bar = Prepared::Progress {
            value: 2.,
            max: 4.,
            label: "2 of 4 tomatoes".into(),
        };
        assert_eq!(
            spoken(&bar, now, &zone, Language::English),
            "2 of 4 tomatoes"
        );
    }
}
