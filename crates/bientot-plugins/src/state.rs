//! `plugins/state.json`: what is installed and in which state (data model, « InstalledPlugin »).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use bientot_plugin_api::{Capability, PluginId};
use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::files;
use crate::trust::Trust;

pub const SCHEMA: u32 = 1;
/// Three traps within this window turn the plugin off until the next start (FR-016).
pub const FAILURE_WINDOW: SignedDuration = SignedDuration::from_mins(10);
pub const MAX_FAILURES: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StateFile {
    pub schema: u32,
    #[serde(default)]
    pub plugins: BTreeMap<PluginId, InstalledPlugin>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub version: String,
    pub trust: Trust,
    /// Path → SHA-256 hex, checked at every load.
    pub files: BTreeMap<String, String>,
    pub manifest_hash: String,
    pub granted: Grant,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<Timestamp>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub card_preference: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_on: Option<Date>,
    /// Its place among the installs, 0 for the first. An update keeps it.
    #[serde(default)]
    pub rank: u32,
    /// `installed` or `upgraded` still has to be delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub capabilities: BTreeSet<Capability>,
    /// The manifest the user accepted; another one asks again (FR-006).
    pub manifest_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Status {
    Active,
    Off,
    FailedUntilRestart {
        reason: String,
    },
    Revoked {
        reason_en: String,
        reason_fr: String,
    },
    FilesChanged,
    UnsupportedApi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Pending {
    Installed,
    Upgraded { from: String },
}

impl InstalledPlugin {
    pub fn is_active(&self) -> bool {
        self.status == Status::Active
    }

    pub fn turn_on(&mut self) -> bool {
        self.set_if(matches!(self.status, Status::Off), Status::Active)
    }

    pub fn turn_off(&mut self) -> bool {
        self.set_if(matches!(self.status, Status::Active), Status::Off)
    }

    /// Records a trap. Returns `true` when this one turned the plugin off until the next start.
    pub fn record_failure(&mut self, at: Timestamp, reason: &str) -> bool {
        self.failures.push(at);
        let excess = self.failures.len().saturating_sub(MAX_FAILURES);
        self.failures.drain(..excess);
        let recent = self
            .failures
            .iter()
            .filter(|t| at.duration_since(**t) < FAILURE_WINDOW)
            .count();
        recent >= MAX_FAILURES
            && self.set_if(
                self.is_active(),
                Status::FailedUntilRestart {
                    reason: reason.to_owned(),
                },
            )
    }

    pub fn revoke(&mut self, reason_en: &str, reason_fr: &str) -> bool {
        let revoked = Status::Revoked {
            reason_en: reason_en.to_owned(),
            reason_fr: reason_fr.to_owned(),
        };
        self.set_if(matches!(self.status, Status::Active | Status::Off), revoked)
    }

    pub fn unrevoke(&mut self) -> bool {
        self.set_if(matches!(self.status, Status::Revoked { .. }), Status::Off)
    }

    pub fn files_changed(&mut self) -> bool {
        self.set_if(
            matches!(self.status, Status::Active | Status::Off),
            Status::FilesChanged,
        )
    }

    pub fn api_unsupported(&mut self) -> bool {
        self.set_if(
            matches!(self.status, Status::Active | Status::Off),
            Status::UnsupportedApi,
        )
    }

    fn set_if(&mut self, allowed: bool, status: Status) -> bool {
        if allowed {
            self.status = status;
        }
        allowed
    }
}

impl StateFile {
    /// Reads the file; a missing file is an empty state. A file that does not parse is an error:
    /// the caller keeps every plugin off rather than forget them.
    pub fn load(path: &Path) -> io::Result<StateFile> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(StateFile {
                schema: SCHEMA,
                plugins: BTreeMap::new(),
            }),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        files::replace(path, &bytes)
    }

    /// At start: a plugin turned off after failures comes back (FR-016).
    pub fn on_start(&mut self) {
        for plugin in self.plugins.values_mut() {
            if matches!(plugin.status, Status::FailedUntilRestart { .. }) {
                plugin.status = Status::Active;
                plugin.failures.clear();
            }
        }
    }

    pub fn any_active(&self) -> bool {
        self.plugins.values().any(InstalledPlugin::is_active)
    }

    /// The rank of the next install.
    pub fn next_rank(&self) -> u32 {
        self.plugins.values().map(|p| p.rank + 1).max().unwrap_or(0)
    }

    /// Which of the plugins that have a card the flyout shows on `today`: the one the user chose
    /// last, otherwise the first installed, never one hidden for the day. All the others, the
    /// hidden ones last, go under « More » (US1, scenarios 4 and 5).
    pub fn cards<'a>(
        &self,
        today: Date,
        with_card: impl IntoIterator<Item = &'a PluginId>,
    ) -> Cards<'a> {
        let mut order: Vec<(bool, bool, u32, &PluginId)> = with_card
            .into_iter()
            .filter_map(|id| {
                let plugin = self.plugins.get(id)?;
                let hidden = plugin.hidden_on == Some(today);
                Some((hidden, !plugin.card_preference, plugin.rank, id))
            })
            .collect();
        order.sort();
        let mut ids = order.into_iter().map(|(hidden, .., id)| (hidden, id));
        let shown = ids.next();
        let (shown, more) = match shown {
            Some((false, id)) => (Some(id), ids.map(|(_, id)| id).collect()),
            first => (
                None,
                first.into_iter().chain(ids).map(|(_, id)| id).collect(),
            ),
        };
        Cards { shown, more }
    }
}

/// The flyout's plugin cards, from [`StateFile::cards`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cards<'a> {
    pub shown: Option<&'a PluginId>,
    pub more: Vec<&'a PluginId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> InstalledPlugin {
        InstalledPlugin {
            version: "0.1.0".into(),
            trust: Trust::Unsigned {
                package_hash: "00".repeat(32),
            },
            files: BTreeMap::from([("plugin.json".into(), "ab".repeat(32))]),
            manifest_hash: "cd".repeat(32),
            granted: Grant {
                capabilities: BTreeSet::from([Capability::ReadDay]),
                manifest_hash: "cd".repeat(32),
            },
            status: Status::Active,
            failures: Vec::new(),
            card_preference: false,
            hidden_on: None,
            rank: 0,
            pending: Some(Pending::Installed),
        }
    }

    fn at(minutes: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + minutes * 60).unwrap()
    }

    #[test]
    fn on_and_off() {
        let mut p = plugin();
        assert!(!p.turn_on());
        assert!(p.turn_off() && p.status == Status::Off);
        assert!(!p.turn_off());
        assert!(p.turn_on() && p.is_active());
    }

    #[test]
    fn three_traps_in_ten_minutes_turn_it_off_until_the_next_start() {
        let mut p = plugin();
        assert!(!p.record_failure(at(0), "trap"));
        assert!(!p.record_failure(at(5), "trap"));
        assert!(
            !p.record_failure(at(11), "trap"),
            "the first one is too old"
        );
        assert!(p.is_active());
        assert!(p.record_failure(at(12), "fuel"));
        assert_eq!(
            p.status,
            Status::FailedUntilRestart {
                reason: "fuel".into()
            }
        );
        assert_eq!(p.failures.len(), MAX_FAILURES);
        assert!(!p.turn_on(), "only a restart brings it back");
        let mut state = StateFile {
            schema: SCHEMA,
            plugins: BTreeMap::from([(PluginId::parse("bientot.focus").unwrap(), p)]),
        };
        state.on_start();
        assert!(state.any_active());
    }

    #[test]
    fn revocation_files_changed_and_api() {
        let mut p = plugin();
        assert!(p.revoke("Malware", "Logiciel malveillant"));
        assert!(!p.turn_on() && !p.files_changed());
        assert!(p.unrevoke() && p.status == Status::Off);
        assert!(p.files_changed() && p.status == Status::FilesChanged);
        let mut q = plugin();
        assert!(q.api_unsupported() && !q.is_active());
    }

    #[test]
    fn the_chosen_card_shows_otherwise_the_first_installed_and_never_a_hidden_one() {
        let today = jiff::civil::date(2026, 11, 9);
        let ids = ["bientot.zebra", "bientot.apple", "bientot.mango"]
            .map(|id| PluginId::parse(id).unwrap());
        let [zebra, apple, mango] = &ids;
        let mut state = StateFile::default();
        for (rank, id) in ids.iter().enumerate() {
            let rank = u32::try_from(rank).unwrap();
            state
                .plugins
                .insert(id.clone(), InstalledPlugin { rank, ..plugin() });
        }
        assert_eq!(state.next_rank(), 3);
        let cards = |state: &StateFile, with: &[&PluginId]| {
            let cards = state.cards(today, with.iter().copied());
            (
                cards.shown.cloned(),
                cards.more.into_iter().cloned().collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            cards(&state, &[apple, mango, zebra]),
            (Some(zebra.clone()), vec![apple.clone(), mango.clone()])
        );
        assert_eq!(
            cards(&state, &[mango, apple]),
            (Some(apple.clone()), vec![mango.clone()]),
            "only plugins that have a card count"
        );
        state.plugins.get_mut(mango).unwrap().card_preference = true;
        assert_eq!(
            cards(&state, &[apple, mango, zebra]),
            (Some(mango.clone()), vec![zebra.clone(), apple.clone()])
        );
        state.plugins.get_mut(mango).unwrap().hidden_on = Some(today);
        assert_eq!(
            cards(&state, &[apple, mango, zebra]),
            (Some(zebra.clone()), vec![apple.clone(), mango.clone()])
        );
        state.plugins.get_mut(zebra).unwrap().hidden_on = Some(today);
        state.plugins.get_mut(apple).unwrap().hidden_on = Some(today);
        assert_eq!(
            cards(&state, &[apple, mango, zebra]),
            (None, vec![mango.clone(), zebra.clone(), apple.clone()])
        );
        assert_eq!(cards(&state, &[apple]).0, None, "hidden for the day only");
        let tomorrow = state.cards(today.tomorrow().unwrap(), [apple]);
        assert_eq!(tomorrow.shown, Some(apple));
        assert_eq!(StateFile::default().next_rank(), 0);
    }

    #[test]
    fn the_file_round_trips_through_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert_eq!(StateFile::load(&path).unwrap().plugins.len(), 0);
        let mut state = StateFile::load(&path).unwrap();
        state
            .plugins
            .insert(PluginId::parse("bientot.focus").unwrap(), plugin());
        state.save(&path).unwrap();
        assert_eq!(StateFile::load(&path).unwrap(), state);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        std::fs::write(&path, b"{").unwrap();
        assert!(StateFile::load(&path).is_err());
    }
}
