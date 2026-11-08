//! The plugins of `specs/003-plugin-runtime` in the app: the `plugins` thread, fed with the
//! published day, and the cards it sends back, kept for the flyout.
//!
//! Nothing here runs when no plugin is on: no thread, no engine (FR-018).

pub mod card;

use std::collections::BTreeMap;
use std::sync::Arc;

use bientot_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use bientot_plugin_api::Capability;
use bientot_plugin_api::view::PreparedButton;
use bientot_plugin_api::{CommandSource, PluginId, PreparedCard};
use bientot_plugins::files::PluginsDir;
use bientot_plugins::host::Clock;
use bientot_plugins::log::LogFiles;
use bientot_plugins::manager;
use bientot_plugins::runtime::{POOL_BYTES, Pool, Runtime, WasmSandbox};
use bientot_plugins::state::StateFile;
use bientot_plugins::thread::{self, HostUpdate, PluginThread, Request};
use bientot_store::data_directory::DataDirectory;
use futures::StreamExt as _;
use gpui::{App, BorrowAppContext as _, Global, SharedString};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::host::Today;
use crate::palette;
use crate::strings::Language;

/// A plugin's card and the name its header shows, in the app language.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginCard {
    pub name: SharedString,
    pub card: PreparedCard,
}

/// The cards of the plugins that show one, for the flyout: the one it shows, and the others
/// under « More » (FR-040).
pub struct PluginCards {
    /// Swapped whole, so that a flyout drawing keeps the set it started with.
    pub cards: Arc<BTreeMap<PluginId, PluginCard>>,
    pub shown: Option<PluginId>,
    pub more: Vec<PluginId>,
    /// The clock and the zone the cards' durations, countdowns and times are drawn with.
    pub clock: Clock,
    pub zone: TimeZone,
}

impl Default for PluginCards {
    fn default() -> Self {
        Self {
            cards: Arc::default(),
            shown: None,
            more: Vec::new(),
            clock: Arc::new(Timestamp::now),
            zone: TimeZone::system(),
        }
    }
}

impl Global for PluginCards {}

impl PluginCards {
    /// The card the flyout shows.
    pub fn shown(&self) -> Option<(&PluginId, &PluginCard)> {
        let id = self.shown.as_ref()?;
        Some((id, self.cards.get(id)?))
    }

    /// The cards under « More », in their order.
    pub fn more(&self) -> impl Iterator<Item = (&PluginId, &PluginCard)> {
        self.more
            .iter()
            .filter_map(|id| Some((id, self.cards.get(id)?)))
    }

    /// The local date, as « Hide for today » counts it.
    pub fn today(&self) -> Date {
        (self.clock)().to_zoned(self.zone.clone()).date()
    }

    /// Orders the cards as `state` says on `today`; without a state, as the screenshots draw
    /// them, by plugin id.
    fn arrange(&mut self, state: Option<&StateFile>) {
        let (shown, more) = match state {
            Some(state) => {
                let cards = state.cards(self.today(), self.cards.keys());
                (
                    cards.shown.cloned(),
                    cards.more.into_iter().cloned().collect(),
                )
            }
            None => {
                let mut ids = self.cards.keys().cloned();
                (ids.next(), ids.collect())
            }
        };
        self.shown = shown;
        self.more = more;
    }

    /// Sets `plugin`'s card, or removes it with `None`, and orders the cards again. Returns
    /// whether anything the flyout shows changed.
    pub fn set(
        &mut self,
        plugin: PluginId,
        card: Option<PluginCard>,
        state: Option<&StateFile>,
    ) -> bool {
        let changed = match card {
            Some(card) => {
                self.cards.get(&plugin) != Some(&card) && {
                    Arc::make_mut(&mut self.cards).insert(plugin, card);
                    true
                }
            }
            None => {
                self.cards.contains_key(&plugin) && {
                    Arc::make_mut(&mut self.cards).remove(&plugin);
                    true
                }
            }
        };
        changed && {
            self.arrange(state);
            true
        }
    }
}

/// The running plugins.
struct Plugins {
    thread: PluginThread,
    dir: PluginsDir,
    state: StateFile,
    /// Some loaded plugin reads the day, through `bt:day@1 current` or `day.refreshed`.
    reads_day: bool,
    /// The app's language, which changes only with a restart.
    language: Language,
    /// Each loaded plugin's name in that language, for its card's header.
    names: BTreeMap<PluginId, SharedString>,
    /// The date the cards were last ordered for.
    today: Option<Date>,
}

impl Global for Plugins {}

/// The folder of the plugins in `directory`.
pub fn folder(directory: &DataDirectory) -> PluginsDir {
    PluginsDir::new(directory.path().join("plugins"))
}

/// Starts the plugins that are on, after the tray is ready (plan, Lifecycle 2 and 3).
pub fn start(directory: &DataDirectory, language: Language, cx: &mut App) {
    cx.set_global(PluginCards::default());
    let dir = folder(directory);
    if !dir.state_path().exists() {
        return;
    }
    let mut state = match StateFile::load(&dir.state_path()) {
        Ok(state) => state,
        Err(error) => {
            diagnose(
                cx,
                "plugins.state_unreadable",
                &[("error", error.to_string().as_str().into())],
            );
            return;
        }
    };
    state.on_start();
    dir.sweep();
    let reserved = palette::reserved_titles();
    let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
    let (specs, problems) = manager::load_active(&dir, &mut state, &reserved);
    for (id, problem) in problems {
        let problem = format!("{problem:?}");
        diagnose(
            cx,
            "plugins.stays_off",
            &[
                ("plugin", id.as_str().into()),
                ("problem", problem.as_str().into()),
            ],
        );
    }
    if specs.is_empty() {
        return;
    }
    let names = specs
        .iter()
        .map(|spec| {
            let manifest = &spec.manifest;
            let name = manifest
                .name
                .get(plugin_language(language), &manifest.locales[0]);
            (manifest.id.clone(), SharedString::from(name.to_owned()))
        })
        .collect();
    let reads_day = specs
        .iter()
        .any(|spec| spec.granted.contains(&Capability::ReadDay));
    let clock: bientot_plugins::host::Clock = Arc::new(Timestamp::now);
    let log = Arc::new(LogFiles::new(
        directory.log_directory().join("plugins"),
        clock.clone(),
    ));
    log.prune();
    let (updates, mut received) = futures::channel::mpsc::unbounded();
    let notify = Box::new(move |update: HostUpdate| {
        let _ = updates.unbounded_send(update);
    });
    let spawned = thread::spawn(
        || WasmSandbox::new(Runtime::new(Pool::new(POOL_BYTES))),
        plugin_language(language),
        log,
        clock,
        notify,
    );
    let thread = match spawned {
        Ok(thread) => thread,
        Err(error) => {
            diagnose(
                cx,
                "plugins.no_thread",
                &[("error", error.to_string().as_str().into())],
            );
            return;
        }
    };
    for spec in specs {
        thread.send(Request::Load(Box::new(spec)));
    }
    cx.set_global(Plugins {
        thread,
        dir,
        state,
        reads_day,
        language,
        names,
        today: None,
    });
    cx.spawn(async move |cx| {
        while let Some(update) = received.next().await {
            cx.update(|cx| apply(update, cx));
        }
    })
    .detach();
    day_changed(cx);
}

/// Sends the published day to the plugins that read it. The thread keeps only the latest, which
/// `bt:day@1 current` answers, and sends `day.refreshed` to the plugins that listen to it on a
/// structural change, at most every 30 s (FR-032).
pub fn day_changed(cx: &mut App) {
    let Some(plugins) = cx.try_global::<Plugins>() else {
        return;
    };
    // A card hidden yesterday comes back after midnight.
    let today = cx.global::<PluginCards>().today();
    if plugins.today != Some(today) {
        cx.global_mut::<Plugins>().today = Some(today);
        rearrange(cx);
    }
    let plugins = cx.global::<Plugins>();
    if !plugins.reads_day {
        return;
    }
    let language = plugin_language(plugins.language);
    let day = cx
        .global_mut::<Today>()
        .live()
        .and_then(|host| host.plugin_day(language));
    if let Some(day) = day {
        cx.global::<Plugins>()
            .thread
            .send(Request::Day(Arc::new(day)));
    }
}

fn apply(update: HostUpdate, cx: &mut App) {
    match update {
        HostUpdate::Card { plugin, card } => set_card(plugin, card, cx),
        HostUpdate::Failed { plugin, error, at } => {
            diagnose(
                cx,
                "plugins.failed",
                &[
                    ("plugin", plugin.as_str().into()),
                    ("error", error.as_str().into()),
                ],
            );
            let plugins = cx.global_mut::<Plugins>();
            if manager::record_failure(&plugins.dir, &mut plugins.state, &plugin, at, &error) {
                plugins.thread.send(Request::Unload(plugin.clone()));
                set_card(plugin, None, cx);
            }
        }
        HostUpdate::Delivered { plugin } => {
            let plugins = cx.global_mut::<Plugins>();
            manager::delivered(&plugins.dir, &mut plugins.state, &plugin);
        }
    }
}

fn set_card(plugin: PluginId, card: Option<PreparedCard>, cx: &mut App) {
    let plugins = cx.global::<Plugins>();
    let card = card.map(|card| PluginCard {
        name: plugins.names.get(&plugin).cloned().unwrap_or_default(),
        card,
    });
    let changed = cx.update_global::<PluginCards, _>(|cards, cx| {
        cards.set(plugin, card, Some(&cx.global::<Plugins>().state))
    });
    if changed {
        cx.refresh_windows();
    }
}

/// Orders the cards again after the state changed.
fn rearrange(cx: &mut App) {
    cx.update_global::<PluginCards, _>(|cards, cx| {
        cards.arrange(Some(&cx.global::<Plugins>().state));
    });
    cx.refresh_windows();
}

/// A button of `plugin`'s card: the plugin gets `command.invoked` (FR-033).
pub fn press(plugin: &PluginId, button: &PreparedButton, cx: &mut App) {
    if let Some(plugins) = cx.try_global::<Plugins>() {
        plugins.thread.send(Request::Command {
            plugin: plugin.clone(),
            command: button.command.clone(),
            arg: button.arg.clone(),
            source: CommandSource::Card,
        });
    }
}

/// « Hide for today » in the « … » menu of `plugin`'s card.
pub fn hide_for_today(plugin: &PluginId, cx: &mut App) {
    let today = cx.global::<PluginCards>().today();
    if cx.has_global::<Plugins>() {
        let plugins = cx.global_mut::<Plugins>();
        manager::hide_card(&plugins.dir, &mut plugins.state, plugin, today);
        rearrange(cx);
    }
}

/// A card picked under « More »: the flyout shows it from now on.
pub fn show(plugin: &PluginId, cx: &mut App) {
    if cx.has_global::<Plugins>() {
        let plugins = cx.global_mut::<Plugins>();
        manager::show_card(&plugins.dir, &mut plugins.state, plugin);
        rearrange(cx);
    }
}

/// Writes `event` to the app's diagnostic log. Only what Bientôt says of a plugin goes there,
/// never the plugin's own log lines (FR-034).
fn diagnose(cx: &mut App, event: &str, properties: &[(&str, DiagnosticValue)]) {
    if let Some(host) = cx.global_mut::<Today>().live() {
        host.log(DiagnosticLogLevel::Warning, event, properties);
    }
}

fn plugin_language(language: Language) -> bientot_plugin_api::Language {
    match language {
        Language::French => bientot_plugin_api::Language::Fr,
        Language::English => bientot_plugin_api::Language::En,
    }
}

#[cfg(test)]
mod tests {
    use bientot_plugins::state::StateFile;

    use super::*;

    fn card(name: &str) -> PluginCard {
        PluginCard {
            name: name.to_owned().into(),
            card: PreparedCard {
                lines: Vec::new(),
                actions: Vec::new(),
            },
        }
    }

    #[test]
    fn a_new_card_takes_its_place_in_the_order_and_a_removed_one_gives_it_up() {
        let [focus, tomato] =
            ["bientot.focus", "bientot.tomato"].map(|id| PluginId::parse(id).unwrap());
        let mut cards = PluginCards::default();
        assert!(cards.set(tomato.clone(), Some(card("Tomate")), None));
        assert!(
            !cards.set(tomato.clone(), Some(card("Tomate")), None),
            "the same card"
        );
        assert!(cards.set(focus.clone(), Some(card("Concentration")), None));
        assert_eq!(cards.shown().map(|(id, _)| id), Some(&focus));
        assert_eq!(
            cards.more().map(|(id, _)| id).collect::<Vec<_>>(),
            [&tomato]
        );

        // The state knows the order of the installs: the first installed shows.
        let mut state: StateFile =
            serde_json::from_value(serde_json::json!({ "schema": 1 })).unwrap();
        state.plugins = [(&tomato, 0), (&focus, 1)]
            .into_iter()
            .map(|(id, rank)| {
                let plugin = serde_json::from_value(serde_json::json!({
                    "version": "1.0.0",
                    "trust": { "kind": "unsigned", "package_hash": "00" },
                    "files": {},
                    "manifest_hash": "00",
                    "granted": { "capabilities": [], "manifest_hash": "00" },
                    "status": { "state": "active" },
                    "rank": rank
                }))
                .unwrap();
                (id.clone(), plugin)
            })
            .collect();
        cards.arrange(Some(&state));
        assert_eq!(cards.shown().map(|(id, _)| id), Some(&tomato));
        assert!(cards.set(tomato.clone(), None, Some(&state)));
        assert_eq!(cards.shown().map(|(id, _)| id), Some(&focus));
        assert_eq!(cards.more().count(), 0);
    }
}
