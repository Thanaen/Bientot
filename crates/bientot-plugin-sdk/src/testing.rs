//! A native host for a plugin's tests. It answers the plugin's host calls as Bientôt does, from
//! the same contracts and the same checks: a card is validated against the manifest and the
//! locale files, a call that needs a capability the manifest does not declare fails the test,
//! and a refused card is returned as a failure.
//!
//! ```no_run
//! use bientot_plugin_sdk::prelude::*;
//! use bientot_plugin_sdk::testing::{TestHost, demo_days};
//! # #[derive(Default)] struct Focus;
//! # impl Plugin for Focus {}
//!
//! let mut host = TestHost::load(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/focus"));
//! let mut plugin = Focus::default();
//! host.start(&mut plugin).unwrap();
//! for (name, day) in demo_days() {
//!     host.refresh(&mut plugin, day).unwrap();
//!     println!("{name}\n{}", host.render());
//! }
//! ```

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use bientot_plugin_api::locale::TextLookup;
use bientot_plugin_api::manifest::EventKind;
use bientot_plugin_api::view::{Prepared, ViewContext};
use bientot_plugin_api::{
    ActivateReason, Capability, Card, CommandSource, DaySnapshot, Event, Language, LocaleTexts,
    Manifest, PreparedCard,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::Plugin;
use crate::host::Import;

/// The limits of `contracts/host-abi.md` the test host enforces.
const MAX_REQUEST_BYTES: usize = 64 << 10;
const MAX_UI_SETS: u32 = 2;
const MAX_LOG_CHARS: usize = 2048;

/// The snapshots of Bientôt's demo scenarios, as the host builds them from the real accounting
/// (on a UTC machine, with the default settings, in French). The scenario names are those of
/// `bientot_core::demo::DemoScenario`.
pub fn demo_days() -> Vec<(String, DaySnapshot)> {
    let days: Vec<Value> =
        serde_json::from_str(include_str!("../demo-days.json")).expect("demo-days.json is a list");
    days.into_iter()
        .map(|entry| {
            let name = entry["scenario"].as_str().unwrap_or_default().to_owned();
            let day = serde_json::from_value(entry["day"].clone()).expect("a demo day reads");
            (name, day)
        })
        .collect()
}

/// One demo day by its scenario name, such as `OnTrack`.
pub fn demo_day(name: &str) -> DaySnapshot {
    demo_days()
        .into_iter()
        .find(|(scenario, _)| scenario == name)
        .map(|(_, day)| day)
        .unwrap_or_else(|| panic!("no demo day named {name}"))
}

/// A card Bientôt refused, which counts as a failure of the plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub Vec<String>);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bientôt refused a card: {}", self.0.join("; "))
    }
}

impl std::error::Error for Refused {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub level: String,
    pub text: String,
}

struct State {
    manifest: Manifest,
    locales: BTreeMap<String, LocaleTexts>,
    language: Language,
    now: Timestamp,
    day: Option<DaySnapshot>,
    card: Option<Card>,
    ui_sets: u32,
    refused: Vec<String>,
    logs: Vec<LogLine>,
}

impl State {
    fn texts(&self) -> TextLookup<'_> {
        TextLookup {
            preferred: self.locales.get(self.language.tag()),
            fallback: &self.locales[self.manifest.fallback_locale()],
        }
    }

    fn needs(&self, import: Import, capability: Capability) {
        assert!(
            self.manifest.has(capability),
            "{} needs `{}` in the manifest's capabilities: Bientôt would refuse the module",
            import.name(),
            capability.name()
        );
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

/// Answers one host call of a native build, for the [`TestHost`] running the plugin.
pub(crate) fn dispatch(import: Import, request: &[u8]) -> Vec<u8> {
    let state = CURRENT.with_borrow(Clone::clone).unwrap_or_else(|| {
        panic!(
            "{} was called outside TestHost: in a native build, run the plugin through a TestHost",
            import.name()
        )
    });
    let mut state = state.borrow_mut();
    assert!(
        request.len() <= MAX_REQUEST_BYTES,
        "{}: a request is larger than 64 KiB, Bientôt would stop the plugin",
        import.name()
    );
    let request: Value = serde_json::from_slice(request).expect("the SDK sends JSON");
    let answer = match import {
        Import::LogWrite => {
            state.logs.push(LogLine {
                level: request["level"].as_str().unwrap_or_default().to_owned(),
                text: request["text"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(MAX_LOG_CHARS)
                    .collect(),
            });
            return Vec::new();
        }
        Import::TimeNow => json!({ "now": state.now.to_string() }),
        Import::DayCurrent => {
            state.needs(import, Capability::ReadDay);
            match &state.day {
                Some(day) => serde_json::to_value(day).expect("a day serializes"),
                None => json!({ "error": "not-ready" }),
            }
        }
        Import::UiSet => {
            state.needs(import, Capability::UiFlyout);
            if state.ui_sets >= MAX_UI_SETS {
                json!({ "error": "at most 2 updates per event" })
            } else {
                state.ui_sets += 1;
                let checked = Card::from_value(request["tree"].clone()).and_then(|card| {
                    let context = ViewContext {
                        texts: state.texts(),
                        manifest: &state.manifest,
                    };
                    card.prepare(&context).map(|_| card)
                });
                match checked {
                    Ok(card) => {
                        state.card = Some(card);
                        json!({ "ok": true })
                    }
                    Err(error) => {
                        state.refused.push(error.to_string());
                        json!({ "error": error.to_string() })
                    }
                }
            }
        }
        Import::UiClear => {
            state.needs(import, Capability::UiFlyout);
            state.card = None;
            return Vec::new();
        }
    };
    serde_json::to_vec(&answer).expect("an answer serializes")
}

/// Runs a plugin's handlers natively, with an injected clock and day.
pub struct TestHost {
    state: Rc<RefCell<State>>,
}

impl TestHost {
    /// The plugin in `dir`: its `plugin.json` and `locales/<tag>.json`. Panics, with the
    /// message Bientôt would show, when one of them would be refused.
    pub fn load(dir: impl AsRef<Path>) -> TestHost {
        let dir = dir.as_ref();
        let read =
            |path: &Path| std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let manifest = read(&dir.join("plugin.json"));
        let manifest = Manifest::parse(&manifest, &[])
            .unwrap_or_else(|e| panic!("plugin.json: {}", e.message(Language::En)));
        let locales = manifest
            .locales
            .iter()
            .map(|tag| {
                let path = dir.join("locales").join(format!("{tag}.json"));
                let texts = LocaleTexts::parse(&read(&path))
                    .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
                (tag.clone(), texts)
            })
            .collect();
        TestHost::new(manifest, locales)
    }

    pub fn new(manifest: Manifest, locales: BTreeMap<String, LocaleTexts>) -> TestHost {
        assert!(
            locales.contains_key(manifest.fallback_locale()),
            "no texts for the first locale `{}`",
            manifest.fallback_locale()
        );
        TestHost {
            state: Rc::new(RefCell::new(State {
                manifest,
                locales,
                language: Language::En,
                now: Timestamp::UNIX_EPOCH,
                day: None,
                card: None,
                ui_sets: 0,
                refused: Vec::new(),
                logs: Vec::new(),
            })),
        }
    }

    /// The app language for the next events and for [`TestHost::render`]. English by default.
    pub fn language(&mut self, language: Language) -> &mut TestHost {
        self.state.borrow_mut().language = language;
        self
    }

    /// Sets the clock.
    pub fn at(&mut self, now: Timestamp) -> &mut TestHost {
        self.state.borrow_mut().now = now;
        self
    }

    /// Moves the clock forward.
    pub fn advance(&mut self, by: SignedDuration) -> &mut TestHost {
        self.state.borrow_mut().now += by;
        self
    }

    pub fn now(&self) -> Timestamp {
        self.state.borrow().now
    }

    pub fn manifest(&self) -> Manifest {
        self.state.borrow().manifest.clone()
    }

    /// Delivers one event as Bientôt does, and returns the cards it refused, if any.
    pub fn send<P: Plugin>(&mut self, plugin: &mut P, event: Event) -> Result<(), Refused> {
        {
            let mut state = self.state.borrow_mut();
            state.ui_sets = 0;
            state.refused.clear();
        }
        let previous = CURRENT.with_borrow_mut(|current| current.replace(self.state.clone()));
        struct Restore(Option<Rc<RefCell<State>>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let previous = self.0.take();
                CURRENT.with_borrow_mut(|current| *current = previous);
            }
        }
        let _restore = Restore(previous);
        plugin.handle(event);
        let refused = std::mem::take(&mut self.state.borrow_mut().refused);
        if refused.is_empty() {
            Ok(())
        } else {
            Err(Refused(refused))
        }
    }

    /// `installed`, then `activate` for a first start.
    pub fn install<P: Plugin>(&mut self, plugin: &mut P) -> Result<(), Refused> {
        let version = self.state.borrow().manifest.version.to_string();
        self.send(plugin, Event::Installed { version })?;
        self.start(plugin)
    }

    /// `activate`, as at every start of Bientôt.
    pub fn start<P: Plugin>(&mut self, plugin: &mut P) -> Result<(), Refused> {
        let language = self.state.borrow().language;
        self.send(
            plugin,
            Event::Activate {
                language,
                reason: ActivateReason::First,
            },
        )
    }

    /// Makes `day` the current day, sets the clock to its calculation time and sends
    /// `day.refreshed` when the manifest listens to it.
    pub fn refresh<P: Plugin>(
        &mut self,
        plugin: &mut P,
        mut day: DaySnapshot,
    ) -> Result<(), Refused> {
        let listens = {
            let mut state = self.state.borrow_mut();
            day.language = state.language;
            state.now = day.computed_at;
            state.day = Some(day.clone());
            state.manifest.events.contains(&EventKind::DayRefreshed)
        };
        if !listens {
            return Ok(());
        }
        self.send(plugin, Event::DayRefreshed { day: Box::new(day) })
    }

    /// Runs a command of the manifest from the palette.
    pub fn command<P: Plugin>(
        &mut self,
        plugin: &mut P,
        command: &str,
        arg: Option<&str>,
    ) -> Result<(), Refused> {
        assert!(
            self.state.borrow().manifest.command(command).is_some(),
            "`{command}` is not a command of the manifest"
        );
        self.send(
            plugin,
            Event::CommandInvoked {
                command: command.to_owned(),
                arg: arg.map(str::to_owned),
                source: CommandSource::Palette,
            },
        )
    }

    /// Presses the button `index` of the card's action row.
    pub fn press<P: Plugin>(&mut self, plugin: &mut P, index: usize) -> Result<(), Refused> {
        let card = self.card().expect("there is no card to press a button of");
        let button = card
            .actions
            .get(index)
            .unwrap_or_else(|| panic!("the card has no button {index}"))
            .clone();
        self.send(
            plugin,
            Event::CommandInvoked {
                command: button.command,
                arg: button.arg,
                source: CommandSource::Card,
            },
        )
    }

    /// Changes the app language and sends `language.changed` when the manifest listens to it.
    pub fn change_language<P: Plugin>(
        &mut self,
        plugin: &mut P,
        language: Language,
    ) -> Result<(), Refused> {
        let listens = {
            let mut state = self.state.borrow_mut();
            state.language = language;
            state.manifest.events.contains(&EventKind::LanguageChanged)
        };
        if !listens {
            return Ok(());
        }
        self.send(plugin, Event::LanguageChanged { language })
    }

    /// The card as Bientôt would draw it, its texts in the current language.
    pub fn card(&self) -> Option<PreparedCard> {
        let state = self.state.borrow();
        let card = state.card.as_ref()?;
        let context = ViewContext {
            texts: state.texts(),
            manifest: &state.manifest,
        };
        Some(
            card.prepare(&context)
                .expect("a card that was accepted prepares"),
        )
    }

    /// The card as JSON, as the plugin sent it.
    pub fn tree(&self) -> Option<Value> {
        self.state
            .borrow()
            .card
            .as_ref()
            .map(|card| serde_json::to_value(card).expect("a card serializes"))
    }

    pub fn logs(&self) -> Vec<LogLine> {
        self.state.borrow().logs.clone()
    }

    /// The card as text, for reference files: see [`render_card`]. `(no card)` when there is
    /// none.
    pub fn render(&self) -> String {
        let state = self.state.borrow();
        let name = state.manifest.display_name(state.language).to_owned();
        let now = state.now;
        drop(state);
        match self.card() {
            Some(card) => render_card(&name, &card, now),
            None => NO_CARD.to_owned(),
        }
    }

    /// The capabilities the manifest declares.
    pub fn capabilities(&self) -> BTreeSet<Capability> {
        self.state.borrow().manifest.capabilities.clone()
    }
}

/// What [`TestHost::render`] gives when the plugin shows no card.
pub const NO_CARD: &str = "(no card)\n";

/// A card as text, for reference files: the plugin's `name`, one line per line of the card, then
/// the buttons as `[label](command arg)`. Durations read `h:mm`, instants `hh:mm` in UTC, a
/// growing duration and a countdown are read at `now`.
pub fn render_card(name: &str, card: &PreparedCard, now: Timestamp) -> String {
    let mut out = format!("{name}\n");
    for line in &card.lines {
        let _ = writeln!(out, "  {}", render(line, now));
    }
    if !card.actions.is_empty() {
        let buttons: Vec<String> = card
            .actions
            .iter()
            .map(|b| match &b.arg {
                Some(arg) => format!("[{}]({} {arg})", b.label, b.command),
                None => format!("[{}]({})", b.label, b.command),
            })
            .collect();
        let _ = writeln!(out, "  {}", buttons.join(" "));
    }
    out
}

/// How many times [`demo_report`] presses the first button of the card after the demo days.
pub const REPORT_PRESSES: usize = 3;
/// How far the clock moves before each of those presses.
pub const REPORT_PRESS_STEP: SignedDuration = SignedDuration::from_mins(30);

/// The cards of a plugin over every demo day, in `language`: `installed` and `activate`, then
/// `day.refreshed` with each day in turn, then up to [`REPORT_PRESSES`] presses of the card's
/// first button, the clock [`REPORT_PRESS_STEP`] later each time; the card is rendered after each
/// step. `bientot-plugin check --expect` runs the same sequence on the built module in Bientôt's
/// own runtime and compares, so a sample's reference files prove that its WebAssembly build
/// behaves as its tests do.
pub fn demo_report<P: Plugin + Default>(dir: impl AsRef<Path>, language: Language) -> String {
    let mut host = TestHost::load(dir);
    host.language(language);
    let mut plugin = P::default();
    host.install(&mut plugin)
        .expect("no card is refused at install");
    let mut out = format!("# start\n{}", host.render());
    for (name, day) in demo_days() {
        if let Err(refused) = host.refresh(&mut plugin, day) {
            panic!("{name}: {refused}");
        }
        let _ = write!(out, "# {name}\n{}", host.render());
    }
    for _ in 0..REPORT_PRESSES {
        let Some(button) = host.card().and_then(|card| card.actions.first().cloned()) else {
            break;
        };
        host.advance(REPORT_PRESS_STEP);
        if let Err(refused) = host.press(&mut plugin, 0) {
            panic!("press {}: {refused}", button.label);
        }
        let _ = write!(out, "# press {}\n{}", button.label, host.render());
    }
    out
}

fn render(node: &Prepared, now: Timestamp) -> String {
    match node {
        Prepared::Stack(children) => children
            .iter()
            .map(|c| render(c, now))
            .collect::<Vec<_>>()
            .join(" / "),
        Prepared::Row(children) => children
            .iter()
            .map(|c| render(c, now))
            .collect::<Vec<_>>()
            .join(" "),
        Prepared::Text { text, .. } => text.clone(),
        Prepared::Duration { value, since } => {
            let grown = since.map_or(0, |since| (now.as_second() - since.as_second()).max(0));
            hours(value + grown)
        }
        Prepared::Countdown { until } => {
            format!("⏳{}", hours((until.as_second() - now.as_second()).max(0)))
        }
        Prepared::Time { at } => {
            let time = at.to_zoned(jiff::tz::TimeZone::UTC).time();
            format!("{:02}:{:02}", time.hour(), time.minute())
        }
        Prepared::Badge { text, tone } => {
            format!("({}: {text})", format!("{tone:?}").to_lowercase())
        }
        Prepared::Progress { value, max, label } => {
            let filled = ((value / max) * 10.0).round() as usize;
            format!(
                "{}{} {label}",
                "▰".repeat(filled.min(10)),
                "▱".repeat(10 - filled.min(10))
            )
        }
    }
}

/// `h:mm`.
fn hours(seconds: i64) -> String {
    let minutes = seconds / 60;
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// Compares `actual` with the reference file at `path`, or writes it when `BIENTOT_BLESS` is set.
pub fn assert_reference(path: impl AsRef<Path>, actual: &str) {
    let path = path.as_ref();
    if std::env::var_os("BIENTOT_BLESS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run the test with BIENTOT_BLESS=1 to write it",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "{} differs; run the test with BIENTOT_BLESS=1 to accept the change\n--- expected\n{expected}\n--- actual\n{actual}",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    fn host(capabilities: &str) -> TestHost {
        let manifest = format!(
            r#"{{ "id": "test.sdk", "name": {{ "en": "Test", "fr": "Essai" }}, "version": "1.0.0",
                 "api": "1.0", "locales": ["en", "fr"], "capabilities": [{capabilities}],
                 "events": [], "commands": [{{ "name": "go", "title": {{ "en": "Go" }} }}] }}"#
        );
        let manifest = Manifest::parse(manifest.as_bytes(), &[]).unwrap();
        TestHost::new(
            manifest,
            BTreeMap::from([
                (
                    "en".into(),
                    LocaleTexts::from_pairs([("hi", "Hi {name}"), ("go", "Go")]),
                ),
                (
                    "fr".into(),
                    LocaleTexts::from_pairs([("hi", "Salut {name}")]),
                ),
            ]),
        )
    }

    /// Runs `body` as the plugin's `command`.
    struct Script(fn());

    impl Plugin for Script {
        fn command(&mut self, _: &str, _: Option<&str>, _: CommandSource) {
            (self.0)()
        }
    }

    fn run(host: &mut TestHost, body: fn()) -> Result<(), Refused> {
        host.command(&mut Script(body), "go", None)
    }

    #[test]
    fn a_card_is_drawn_in_the_language_with_its_fallback() {
        let mut host = host(r#""ui:flyout""#);
        run(&mut host, || {
            ui::show(
                card()
                    .line(text("hi").arg("name", "Ana"))
                    .action(button("go", text("go"))),
            )
            .unwrap()
        })
        .unwrap();
        assert_eq!(host.render(), "Test\n  Hi Ana\n  [Go](go)\n");
        host.language(Language::Fr);
        assert_eq!(host.render(), "Essai\n  Salut Ana\n  [Go](go)\n");
    }

    #[test]
    fn a_refused_card_is_a_failure_and_the_previous_one_stays() {
        let mut host = host(r#""ui:flyout""#);
        run(&mut host, || ui::show(card().line(text("hi"))).unwrap()).unwrap();
        let refused = run(&mut host, || {
            let error = ui::show(card().line(text("missing"))).unwrap_err();
            assert!(error.contains("missing"));
        })
        .unwrap_err();
        assert!(refused.0[0].contains("`missing`"));
        assert!(host.render().contains("Hi {name}"));
    }

    #[test]
    fn a_third_card_in_one_event_is_refused() {
        let mut host = host(r#""ui:flyout""#);
        run(&mut host, || {
            ui::show(card().line(text("hi"))).unwrap();
            ui::show(card().line(text("hi"))).unwrap();
            assert!(ui::show(card().line(text("hi"))).is_err());
        })
        .unwrap();
    }

    #[test]
    #[should_panic(expected = "needs `read:day`")]
    fn reading_the_day_without_the_capability_fails_the_test() {
        let mut host = host(r#""ui:flyout""#);
        let _ = run(&mut host, || {
            day::current();
        });
    }

    #[test]
    fn the_day_and_the_clock_are_those_of_the_host() {
        let mut host = host(r#""read:day""#);
        host.at("2026-11-09T08:00:00Z".parse().unwrap());
        run(&mut host, || {
            assert_eq!(day::current(), None);
            assert_eq!(time::now().to_string(), "2026-11-09T08:00:00Z");
            log::warn("hello");
        })
        .unwrap();
        assert_eq!(host.logs()[0].text, "hello");
        let day = demo_day("OnTrack");
        host.refresh(&mut Script(|| {}), day.clone()).unwrap();
        assert_eq!(host.now(), day.computed_at);
        run(&mut host, || assert!(day::current().is_some())).unwrap();
    }

    #[test]
    #[should_panic(expected = "outside TestHost")]
    fn a_host_call_outside_a_test_host_panics() {
        time::now();
    }

    #[test]
    fn every_demo_scenario_has_a_day() {
        assert_eq!(demo_days().len(), 13);
    }
}
