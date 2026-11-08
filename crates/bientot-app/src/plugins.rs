//! The plugins of `specs/003-plugin-runtime` in the app: the `plugins` thread, fed with the
//! published day, and the cards it sends back, kept for the flyout.
//!
//! Nothing here runs when no plugin is on: no thread, no engine (FR-018).

pub mod card;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use bientot_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use bientot_plugin_api::Capability;
use bientot_plugin_api::manifest::DEFAULT_MEMORY;
use bientot_plugin_api::view::PreparedButton;
use bientot_plugin_api::{CommandSource, PluginId, PreparedCard};
use bientot_plugins::files::PluginsDir;
use bientot_plugins::host::Clock;
use bientot_plugins::log::LogFiles;
use bientot_plugins::manager::{self, InstallError, Offer};
use bientot_plugins::package;
use bientot_plugins::runtime::{POOL_BYTES, Pool, Runtime, WasmSandbox};
use bientot_plugins::state::{StateFile, Status};
use bientot_plugins::thread::{self, HostUpdate, PluginSpec, PluginThread, Request};
use bientot_plugins::trust::{self, Trust};
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

/// The plugins of the user's data folder: what is installed, and the `plugins` thread once a
/// plugin is on.
struct Plugins {
    dir: PluginsDir,
    log_directory: PathBuf,
    state: StateFile,
    /// The app's language, which changes only with a restart.
    language: Language,
    /// What « Paramètres › Plugins » lists, read again after each change of the state.
    installed: Arc<Vec<Installed>>,
    running: Option<Running>,
    /// The date the cards were last ordered for.
    today: Option<Date>,
}

/// The `plugins` thread and the plugins it holds.
struct Running {
    thread: PluginThread,
    loaded: BTreeMap<PluginId, Loaded>,
}

struct Loaded {
    /// The plugin's name in the app's language, for its card's header.
    name: SharedString,
    /// It reads the day, through `bt:day@1 current` or `day.refreshed`.
    reads_day: bool,
}

impl Global for Plugins {}

/// An installed plugin, as « Paramètres › Plugins » shows it (FR-044).
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: PluginId,
    /// In the app's language; the id when the manifest cannot be read.
    pub name: String,
    pub version: String,
    /// The author's key fingerprint, `None` for an unsigned plugin.
    pub fingerprint: Option<String>,
    pub capabilities: Vec<Capability>,
    pub status: Status,
}

/// The folder of the plugins in `directory`.
pub fn folder(directory: &DataDirectory) -> PluginsDir {
    PluginsDir::new(directory.path().join("plugins"))
}

/// Reads the installed plugins and starts those that are on, after the tray is ready (plan,
/// Lifecycle 2 and 3). With none on, nothing more than the read happens.
pub fn start(directory: &DataDirectory, language: Language, cx: &mut App) {
    cx.set_global(PluginCards::default());
    let dir = folder(directory);
    let state = if dir.state_path().exists() {
        match StateFile::load(&dir.state_path()) {
            Ok(mut state) => {
                state.on_start();
                dir.sweep();
                state
            }
            Err(error) => {
                diagnose(
                    cx,
                    "plugins.state_unreadable",
                    &[("error", error.to_string().as_str().into())],
                );
                return;
            }
        }
    } else {
        StateFile::default()
    };
    let mut plugins = Plugins {
        dir,
        log_directory: directory.log_directory().join("plugins"),
        state,
        language,
        installed: Arc::default(),
        running: None,
        today: None,
    };
    let reserved = palette::reserved_titles();
    let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
    let (specs, problems) = manager::load_active(&plugins.dir, &mut plugins.state, &reserved);
    plugins.list();
    cx.set_global(plugins);
    for (id, problem) in problems {
        stays_off(&id, &problem, cx);
    }
    for spec in specs {
        load(spec, cx);
    }
    day_changed(cx);
}

impl Plugins {
    /// Reads again what the settings page lists.
    fn list(&mut self) {
        let language = plugin_language(self.language);
        self.installed = Arc::new(
            self.state
                .plugins
                .iter()
                .map(|(id, plugin)| {
                    let manifest = manager::installed_manifest(&self.dir, id);
                    Installed {
                        id: id.clone(),
                        name: manifest.as_ref().map_or_else(
                            || id.as_str().to_owned(),
                            |m| m.name.get(language, &m.locales[0]).to_owned(),
                        ),
                        version: plugin.version.clone(),
                        fingerprint: match &plugin.trust {
                            Trust::Signed { key, .. } => {
                                package::parse_key(key).map(|key| package::fingerprint(&key))
                            }
                            Trust::Unsigned { .. } => None,
                        },
                        capabilities: plugin.granted.capabilities.iter().copied().collect(),
                        status: plugin.status.clone(),
                    }
                })
                .collect(),
        );
    }

    fn reads_day(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.loaded.values().any(|plugin| plugin.reads_day))
    }
}

/// Hands `spec` to the `plugins` thread, started with the first plugin that loads.
fn load(spec: PluginSpec, cx: &mut App) {
    if cx.global::<Plugins>().running.is_none() && !run(cx) {
        return;
    }
    let plugins = cx.global_mut::<Plugins>();
    let manifest = &spec.manifest;
    let name = manifest
        .name
        .get(plugin_language(plugins.language), &manifest.locales[0]);
    let loaded = Loaded {
        name: SharedString::from(name.to_owned()),
        reads_day: spec.granted.contains(&Capability::ReadDay),
    };
    let running = plugins.running.as_mut().expect("the thread runs");
    running.loaded.insert(manifest.id.clone(), loaded);
    running.thread.send(Request::Load(Box::new(spec)));
}

/// Starts the `plugins` thread and drains its updates. Returns whether it runs.
fn run(cx: &mut App) -> bool {
    let plugins = cx.global::<Plugins>();
    let clock: Clock = Arc::new(Timestamp::now);
    let log = Arc::new(LogFiles::new(plugins.log_directory.clone(), clock.clone()));
    log.prune();
    let (updates, mut received) = futures::channel::mpsc::unbounded();
    let notify = Box::new(move |update: HostUpdate| {
        let _ = updates.unbounded_send(update);
    });
    let spawned = thread::spawn(
        || WasmSandbox::new(Runtime::new(Pool::new(POOL_BYTES))),
        plugin_language(plugins.language),
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
            return false;
        }
    };
    cx.global_mut::<Plugins>().running = Some(Running {
        thread,
        loaded: BTreeMap::new(),
    });
    cx.spawn(async move |cx| {
        while let Some(update) = received.next().await {
            cx.update(|cx| apply(update, cx));
        }
    })
    .detach();
    true
}

/// Takes `id` out of the thread and its card out of the flyout.
fn unload(id: &PluginId, cx: &mut App) {
    if let Some(running) = cx.global_mut::<Plugins>().running.as_mut()
        && running.loaded.remove(id).is_some()
    {
        running.thread.send(Request::Unload(id.clone()));
    }
    set_card(id.clone(), None, cx);
}

fn stays_off(id: &PluginId, problem: &manager::LoadProblem, cx: &mut App) {
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

/// The state changed: the settings page lists it again.
fn changed(cx: &mut App) {
    cx.global_mut::<Plugins>().list();
    cx.refresh_windows();
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
    if !plugins.reads_day() {
        return;
    }
    let language = plugin_language(plugins.language);
    let day = cx
        .global_mut::<Today>()
        .live()
        .and_then(|host| host.plugin_day(language));
    if let (Some(day), Some(running)) = (day, &cx.global::<Plugins>().running) {
        running.thread.send(Request::Day(Arc::new(day)));
    }
}

fn apply(update: HostUpdate, cx: &mut App) {
    match update {
        HostUpdate::Card { plugin, card } => {
            // A card on its way when the plugin was turned off stays out.
            let loaded = cx
                .global::<Plugins>()
                .running
                .as_ref()
                .is_some_and(|running| running.loaded.contains_key(&plugin));
            if loaded || card.is_none() {
                set_card(plugin, card, cx);
            }
        }
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
                unload(&plugin, cx);
                changed(cx);
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
        name: plugins
            .running
            .as_ref()
            .and_then(|running| running.loaded.get(&plugin))
            .map(|loaded| loaded.name.clone())
            .unwrap_or_default(),
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
    if let Some(running) = cx
        .try_global::<Plugins>()
        .and_then(|plugins| plugins.running.as_ref())
    {
        running.thread.send(Request::Command {
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

/// What « Paramètres › Plugins » lists; `None` without plugins: in a demo, or when
/// `state.json` cannot be read.
pub fn installed(cx: &App) -> Option<Arc<Vec<Installed>>> {
    cx.try_global::<Plugins>()
        .map(|plugins| plugins.installed.clone())
}

/// Turns `id` on, which loads it, or off, which takes its card and commands away at once.
pub fn turn(id: &PluginId, on: bool, cx: &mut App) {
    if !cx.has_global::<Plugins>() {
        return;
    }
    let plugins = cx.global_mut::<Plugins>();
    if !manager::turn(&plugins.dir, &mut plugins.state, id, on) {
        return;
    }
    if on {
        load_installed(id, cx);
    } else {
        unload(id, cx);
    }
    changed(cx);
}

/// Removes `id`: its folder and its state go, and nothing it showed remains.
pub fn remove(id: &PluginId, cx: &mut App) -> std::io::Result<()> {
    if !cx.has_global::<Plugins>() {
        return Ok(());
    }
    unload(id, cx);
    let plugins = cx.global_mut::<Plugins>();
    let removed = manager::remove(&plugins.dir, &mut plugins.state, id);
    changed(cx);
    removed
}

/// Checks and loads `id`, installed or turned on just now.
fn load_installed(id: &PluginId, cx: &mut App) {
    let reserved = palette::reserved_titles();
    let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
    let plugins = cx.global_mut::<Plugins>();
    match manager::load(&plugins.dir, &mut plugins.state, id, &reserved) {
        Some(Ok(spec)) => {
            load(spec, cx);
            day_changed(cx);
        }
        Some(Err(problem)) => stays_off(id, &problem, cx),
        None => {}
    }
}

/// Why a package was not offered for install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The file cannot be read.
    Unreadable,
    /// What is wrong with the package, in the app's language.
    Package(String),
    /// The module is not one this Bientôt runs.
    Module,
    /// The installed plugin is signed by `bound`, this package by `offered`.
    OtherAuthor { bound: String, offered: String },
    /// The installed plugin is signed by `bound` and this package is not.
    NowUnsigned { bound: String },
}

/// A package that passed every check.
pub struct Offered {
    /// `None` in a [`Offered::preview`].
    offer: Option<Offer>,
    pub consent: Consent,
}

impl Offered {
    /// What the consent dialog shows of a package that is not there, for the screenshots and
    /// the tests: installing it fails.
    pub fn preview(consent: Consent) -> Self {
        Self {
            offer: None,
            consent,
        }
    }
}

/// What the consent dialog says of a package (FR-007), in the app's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    pub name: String,
    pub version: String,
    /// The version it replaces.
    pub replaces: Option<String>,
    pub fingerprint: Option<String>,
    pub capabilities: Vec<Capability>,
    /// The memory it declares, in MiB, when above the default.
    pub memory_mib: Option<u32>,
    /// The same author ships a manifest that asks for nothing more: no dialog (FR-006).
    pub needs_consent: bool,
}

impl std::fmt::Debug for Offered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Offered")
            .field("consent", &self.consent)
            .finish_non_exhaustive()
    }
}

/// Reads and checks the package `file` off the main thread, then hands `done` the offer or why
/// there is none. `None` without plugins.
pub fn read_package(
    file: PathBuf,
    done: impl FnOnce(Result<Offered, Refused>, &mut App) + 'static,
    cx: &mut App,
) {
    let Some(plugins) = cx.try_global::<Plugins>() else {
        return;
    };
    let state = plugins.state.clone();
    let language = plugin_language(plugins.language);
    let read = cx.background_executor().spawn(async move {
        let bytes = std::fs::read(&file).map_err(|_| Refused::Unreadable)?;
        let reserved = palette::reserved_titles();
        let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
        let offer =
            manager::offer(&bytes, &reserved, &Runtime::default(), &state).map_err(|error| {
                match error {
                    InstallError::Package(error) => Refused::Package(error.message(language)),
                    InstallError::Module(_) => Refused::Module,
                    InstallError::Trust(trust::Refusal::OtherAuthor { bound, offered }) => {
                        Refused::OtherAuthor { bound, offered }
                    }
                    InstallError::Trust(trust::Refusal::NowUnsigned { bound }) => {
                        Refused::NowUnsigned { bound }
                    }
                }
            })?;
        let manifest = &offer.package.manifest;
        let consent = Consent {
            name: manifest.name.get(language, &manifest.locales[0]).to_owned(),
            version: manifest.version.to_string(),
            replaces: offer.replaces.clone(),
            fingerprint: offer.fingerprint(),
            capabilities: offer.capabilities().iter().copied().collect(),
            memory_mib: (manifest.memory > DEFAULT_MEMORY).then_some(manifest.memory >> 20),
            needs_consent: offer.needs_consent(&state),
        };
        Ok(Offered {
            offer: Some(offer),
            consent,
        })
    });
    cx.spawn(async move |cx| {
        let read = read.await;
        cx.update(|cx| done(read, cx));
    })
    .detach();
}

/// Installs the package the user accepted and loads it. Returns its id.
pub fn install(offered: Offered, cx: &mut App) -> std::io::Result<PluginId> {
    let (true, Some(offer)) = (cx.has_global::<Plugins>(), offered.offer) else {
        return Err(std::io::Error::other("nothing to install"));
    };
    let id = offer.package.manifest.id.clone();
    // An unsigned replacement starts over, and a plugin that changed must load anew.
    unload(&id, cx);
    let plugins = cx.global_mut::<Plugins>();
    let installed = manager::install(&plugins.dir, &mut plugins.state, offer);
    // After a failed update, the version still installed loads again.
    load_installed(&id, cx);
    changed(cx);
    installed
}

/// The plugins folder, where File Explorer shows it: under MSIX the files Bientôt wrote sit
/// in the package's `LocalCache`, which only the folder's final path leads to.
pub fn open_folder(cx: &mut App) {
    let Some(plugins) = cx.try_global::<Plugins>() else {
        return;
    };
    let root = plugins.dir.root().to_path_buf();
    if std::fs::create_dir_all(&root).is_err() {
        return;
    }
    #[cfg(windows)]
    let root = bientot_windows::diagnostics::final_directory(&root).unwrap_or(root);
    cx.open_with_system(&root);
}

/// Writes `event` to the app's diagnostic log. Only what Bientôt says of a plugin goes there,
/// never the plugin's own log lines (FR-034).
fn diagnose(cx: &mut App, event: &str, properties: &[(&str, DiagnosticValue)]) {
    if let Some(host) = cx.global_mut::<Today>().live() {
        host.log(DiagnosticLogLevel::Warning, event, properties);
    }
}

pub(crate) fn plugin_language(language: Language) -> bientot_plugin_api::Language {
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
