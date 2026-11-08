//! The flyout with the cards of the sample plugins, their texts read from the samples' locale
//! files:
//! - `flyout-plugin-focus.png`, « Concentration » after the counted time;
//! - `flyout-plugin-menu.png`, its « … » menu open, the other cards under it;
//! - `flyout-plugin-tomato.png`, a countdown and two buttons;
//! - `flyout-plugin-battery.png`, a progress bar, a badge and a time;
//! - `flyout-plugins-paused.png`, the plugins off after two crashed starts.

use std::collections::BTreeMap;
use std::sync::Arc;

use bientot_app::demo::{self, DemoHost};
use bientot_app::plugins::{PluginCard, PluginCards};
use bientot_app::strings::Language;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::TrackingState;
use bientot_plugin_api::PluginId;
use bientot_plugin_api::view::{Prepared, PreparedButton, PreparedCard, Role, Tone};
use gpui::{HeadlessAppContext, SharedString};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::capture;
use super::shared::{open_flyout, working};

/// The drawing's clock: 14:00 in Paris.
const NOW: &str = "2026-09-15T12:00:00Z";

/// A sample plugin's texts in `language`, from its locale file.
struct Texts(serde_json::Map<String, serde_json::Value>);

impl Texts {
    fn of(plugin: &str, language: Language) -> Self {
        let json = match (plugin, language) {
            ("focus", Language::French) => {
                include_str!("../../../../bientot-plugin-sdk/examples/focus/locales/fr.json")
            }
            ("focus", Language::English) => {
                include_str!("../../../../bientot-plugin-sdk/examples/focus/locales/en.json")
            }
            ("tomato", Language::French) => {
                include_str!("../../../../bientot-plugin-sdk/examples/tomato/locales/fr.json")
            }
            ("tomato", Language::English) => {
                include_str!("../../../../bientot-plugin-sdk/examples/tomato/locales/en.json")
            }
            ("battery", Language::French) => {
                include_str!("../../../../bientot-plugin-sdk/examples/battery/locales/fr.json")
            }
            ("battery", Language::English) => {
                include_str!("../../../../bientot-plugin-sdk/examples/battery/locales/en.json")
            }
            ("weekend", Language::French) => {
                include_str!("../../../../bientot-plugin-sdk/examples/weekend/locales/fr.json")
            }
            ("weekend", Language::English) => {
                include_str!("../../../../bientot-plugin-sdk/examples/weekend/locales/en.json")
            }
            _ => unreachable!("no sample plugin {plugin}"),
        };
        Texts(serde_json::from_str(json).expect("a sample locale file is JSON"))
    }

    fn get(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.0[key]
            .as_str()
            .expect("the sample has the key")
            .to_owned();
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    fn text(&self, key: &str, role: Role) -> Prepared {
        Prepared::Text {
            text: self.get(key, &[]),
            role,
        }
    }

    fn button(&self, command: &str, key: &str) -> PreparedButton {
        PreparedButton {
            command: command.into(),
            arg: None,
            label: self.get(key, &[]),
        }
    }
}

fn at(text: &str) -> Timestamp {
    text.parse().expect("a valid instant")
}

fn minutes(minutes: i64) -> Timestamp {
    at(NOW) + jiff::SignedDuration::from_mins(minutes)
}

/// The four sample plugins' cards, as they draw them on the day of the screenshots.
fn sample_cards(language: Language) -> Vec<(&'static str, PluginCard)> {
    let name = |en: &str, fr: &str| {
        SharedString::from(match language {
            Language::French => fr.to_owned(),
            Language::English => en.to_owned(),
        })
    };
    let focus = Texts::of("focus", language);
    let tomato = Texts::of("tomato", language);
    let battery = Texts::of("battery", language);
    let weekend = Texts::of("weekend", language);
    vec![
        (
            "bientot.focus",
            PluginCard {
                name: name("Focus", "Concentration"),
                card: PreparedCard {
                    lines: vec![
                        Prepared::Row(vec![
                            focus.text("longest", Role::Body),
                            Prepared::Duration {
                                value: 5400,
                                since: None,
                            },
                        ]),
                        Prepared::Row(vec![
                            focus.text("current", Role::Body),
                            Prepared::Duration {
                                value: 0,
                                since: Some(minutes(-42)),
                            },
                        ]),
                    ],
                    actions: vec![focus.button("toggle-detail", "more")],
                },
            },
        ),
        (
            "bientot.tomato",
            PluginCard {
                name: name("Tomato", "Tomate"),
                card: PreparedCard {
                    lines: vec![Prepared::Row(vec![
                        tomato.text("focus", Role::Body),
                        Prepared::Countdown { until: minutes(18) },
                    ])],
                    actions: vec![
                        tomato.button("break", "short-break"),
                        tomato.button("stop", "stop"),
                    ],
                },
            },
        ),
        (
            "bientot.battery",
            PluginCard {
                name: name("Battery", "Batterie"),
                card: PreparedCard {
                    lines: vec![
                        Prepared::Progress {
                            value: 34.,
                            max: 100.,
                            label: battery.get("charge", &[("percent", "34")]),
                        },
                        Prepared::Badge {
                            text: battery.get("low", &[]),
                            tone: Tone::Warning,
                        },
                        Prepared::Row(vec![
                            battery.text("recharge", Role::Muted),
                            Prepared::Time { at: minutes(175) },
                        ]),
                    ],
                    actions: vec![battery.button("coffee", "coffee")],
                },
            },
        ),
        (
            "bientot.weekend",
            PluginCard {
                name: name("Weekend", "Vivement le week-end"),
                card: PreparedCard {
                    lines: vec![
                        Prepared::Text {
                            text: weekend.get("days-left", &[("count", "3")]),
                            role: Role::Title,
                        },
                        weekend.text("even", Role::Muted),
                    ],
                    actions: Vec::new(),
                },
            },
        ),
    ]
}

/// The sample cards, `shown` drawn and the others under « Autres cartes ».
fn cards(language: Language, shown: &str) -> PluginCards {
    let all = sample_cards(language);
    let mut cards = PluginCards {
        clock: Arc::new(|| at(NOW)),
        zone: TimeZone::get("Europe/Paris").expect("the zone database has Paris"),
        ..PluginCards::default()
    };
    let map: BTreeMap<PluginId, PluginCard> = all
        .iter()
        .map(|(id, card)| (PluginId::parse(id).expect("a valid id"), card.clone()))
        .collect();
    cards.cards = Arc::new(map);
    let shown = PluginId::parse(shown).expect("a valid id");
    cards.more = all
        .iter()
        .map(|(id, _)| PluginId::parse(id).expect("a valid id"))
        .filter(|id| *id != shown)
        .collect();
    cards.shown = Some(shown);
    cards
}

pub fn capture_cards(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for (slug, shown, menu) in [
        ("plugin-focus", Some("bientot.focus"), false),
        ("plugin-menu", Some("bientot.focus"), true),
        ("plugin-tomato", Some("bientot.tomato"), false),
        ("plugin-battery", Some("bientot.battery"), false),
        ("plugins-paused", None, false),
    ] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let day = day?;
        let shown = match shown {
            Some(shown) => cards(language, shown),
            None => PluginCards {
                paused: true,
                ..PluginCards::default()
            },
        };
        cx.update(|cx| cx.set_global(shown));
        let window = open_flyout(cx, day, working(TrackingState::Running), None)?;
        if menu {
            cx.update(|cx| {
                window.update(cx, |flyout, window, cx| flyout.open_plugin_menu(window, cx))
            })?;
            cx.advance_clock(std::time::Duration::from_millis(400));
        }
        capture(
            cx,
            window.into(),
            &output.join(format!("flyout-{slug}.png")),
        )?;
    }
    cx.update(|cx| cx.set_global(PluginCards::default()));
    Ok(())
}
