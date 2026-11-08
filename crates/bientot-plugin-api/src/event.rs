//! The events `bt_handle` receives (contract `host-abi.md`, « Events »).

use serde::{Deserialize, Serialize};

use crate::snapshot::DaySnapshot;
use crate::text::Language;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    /// Once, before the first `activate` after install.
    #[serde(rename = "installed")]
    Installed { version: String },
    /// Once, after an update.
    #[serde(rename = "upgraded")]
    Upgraded { from: String, version: String },
    /// At every instantiation, before any other event of that instance.
    #[serde(rename = "activate")]
    Activate {
        language: Language,
        reason: ActivateReason,
    },
    /// A structural change of the day, at most once per 30 s.
    #[serde(rename = "day.refreshed")]
    DayRefreshed { day: Box<DaySnapshot> },
    #[serde(rename = "command.invoked")]
    CommandInvoked {
        command: String,
        arg: Option<String>,
        source: CommandSource,
    },
    #[serde(rename = "language.changed")]
    LanguageChanged { language: Language },
    /// An event of a later api. A plugin ignores it.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivateReason {
    /// The first instance since Bientôt started.
    First,
    /// The previous instance was idle for 5 minutes.
    Evicted,
    /// The previous instance grew too large and was replaced between events.
    Recreated,
    /// The previous instance trapped.
    Restart,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandSource {
    Palette,
    Card,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_1_0_events_still_read() {
        let events: Vec<Event> =
            serde_json::from_str(include_str!("../tests/fixtures/api-1.0/events.json")).unwrap();
        assert_eq!(events.len(), 6);
        assert_eq!(
            events[3],
            Event::CommandInvoked {
                command: "toggle-range".into(),
                arg: Some("week".into()),
                source: CommandSource::Card
            }
        );
        let day = include_str!("../tests/fixtures/api-1.0/day-snapshot.json");
        let refreshed: Event =
            serde_json::from_str(&format!(r#"{{ "event": "day.refreshed", "day": {day} }}"#))
                .unwrap();
        assert!(matches!(refreshed, Event::DayRefreshed { .. }));
    }

    #[test]
    fn later_events_read_as_unknown() {
        let event: Event =
            serde_json::from_str(r#"{ "event": "timer.fired", "name": "tea" }"#).unwrap();
        assert_eq!(event, Event::Unknown);
    }

    #[test]
    fn events_serialize_with_their_tag() {
        let json = serde_json::to_string(&Event::Activate {
            language: Language::Fr,
            reason: ActivateReason::First,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"event":"activate","language":"fr","reason":"first"}"#
        );
    }
}
