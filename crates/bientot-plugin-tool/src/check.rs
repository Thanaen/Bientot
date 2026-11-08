//! `check`: installs a package as Bientôt would and runs it in Bientôt's own runtime, with the
//! same fuel and time limits, on the sequence of [`bientot_plugin_sdk::testing::demo_report`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bientot_plugin_api::view::ViewContext;
use bientot_plugin_api::{ActivateReason, CommandSource, Event, Language, PluginId, PreparedCard};
use bientot_plugin_sdk::testing::{
    NO_CARD, REPORT_PRESS_STEP, REPORT_PRESSES, demo_days, render_card,
};
use bientot_plugins::host::{LogLevel, LogSink, PluginContext, UiChange};
use bientot_plugins::manager;
use bientot_plugins::package::MODULE;
use bientot_plugins::runtime::{CallKind, Limits, PluginInstance, Runtime};
use bientot_plugins::state::StateFile;
use jiff::Timestamp;

use crate::{Args, read};

pub fn run(args: &Args) -> Result<(), String> {
    let path = args.path(0, "package")?;
    let bytes = read(&path)?;
    let runtime = Runtime::default();
    let offer =
        manager::offer(&bytes, &[], &runtime, &StateFile::default()).map_err(|e| e.to_string())?;
    let package = &offer.package;
    let manifest = &package.manifest;
    let languages = match args.option("--language") {
        Some("fr") => vec![Language::Fr],
        Some("en") => vec![Language::En],
        Some(other) => return Err(format!("--language takes en or fr, not {other}")),
        None => vec![Language::En, Language::Fr],
    };
    println!(
        "{} {} « {} », {}",
        manifest.id.as_str(),
        manifest.version,
        manifest.display_name(Language::En),
        match offer.fingerprint() {
            Some(print) => format!("signed by {print}"),
            None => "unsigned".to_owned(),
        }
    );
    for capability in offer.capabilities() {
        println!(
            "  asks: {} ({})",
            capability.consent(Language::En),
            capability.name()
        );
    }
    let module = runtime
        .compile(package.module(), &manifest.capabilities)
        .map_err(|e| e.to_string())?;
    println!(
        "  {MODULE}: {} KiB, memory {} KiB reserved, {} KiB at most",
        package.module().len().div_ceil(1024),
        manifest.memory_min / 1024,
        manifest.memory / 1024
    );
    let mut failed = false;
    for language in languages {
        let mut run = Run::new(&runtime, &module, package, language, args.flag("--log"))?;
        let report = run.report()?;
        println!(
            "  {}: {} events, most fuel {:.1} M of {} M, longest {} ms, memory {} KiB",
            language.tag(),
            run.events,
            run.most_fuel as f64 / 1e6,
            bientot_plugins::runtime::BASE_ALLOWANCE / 1_000_000,
            run.longest.as_millis(),
            run.instance.memory_bytes() / 1024,
        );
        match args.option("--expect") {
            Some(dir) => {
                let expected_path =
                    std::path::Path::new(dir).join(format!("{}.txt", language.tag()));
                let expected = String::from_utf8(read(&expected_path)?)
                    .map_err(|_| format!("{} is not text", expected_path.display()))?;
                if let Some(difference) = first_difference(&expected, &report) {
                    eprintln!(
                        "  {}: the module differs from the reference at line {difference}",
                        expected_path.display()
                    );
                    failed = true;
                } else {
                    println!("  {}: matches", expected_path.display());
                }
            }
            None => print!("{report}"),
        }
    }
    if failed {
        Err("the module does not behave as its tests".to_owned())
    } else {
        Ok(())
    }
}

/// The 1-based line where two reports part, with both lines.
fn first_difference(expected: &str, actual: &str) -> Option<String> {
    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    let mut line = 1;
    loop {
        match (expected_lines.next(), actual_lines.next()) {
            (None, None) => return None,
            (a, b) if a == b => line += 1,
            (a, b) => {
                return Some(format!(
                    "{line}:\n    expected {}\n    got      {}",
                    a.unwrap_or("(end)"),
                    b.unwrap_or("(end)")
                ));
            }
        }
    }
}

struct Log {
    print: bool,
}

impl LogSink for Log {
    fn write(&self, plugin: &PluginId, level: LogLevel, text: &str) {
        if self.print {
            eprintln!("  [{} {level:?}] {text}", plugin.as_str());
        }
    }

    fn dropped(&self, plugin: &PluginId, count: u32) {
        eprintln!("  [{}] {count} log lines dropped", plugin.as_str());
    }

    fn host(&self, plugin: &PluginId, text: &str) {
        eprintln!("  [{} host] {text}", plugin.as_str());
    }
}

struct Run<'a> {
    package: &'a bientot_plugins::package::Package,
    language: Language,
    instance: PluginInstance,
    clock: Arc<Mutex<Timestamp>>,
    card: Option<bientot_plugin_api::Card>,
    events: usize,
    most_fuel: u64,
    longest: Duration,
}

impl<'a> Run<'a> {
    fn new(
        runtime: &Runtime,
        module: &bientot_plugins::runtime::Module,
        package: &'a bientot_plugins::package::Package,
        language: Language,
        print_log: bool,
    ) -> Result<Run<'a>, String> {
        let clock = Arc::new(Mutex::new(Timestamp::UNIX_EPOCH));
        let now = clock.clone();
        let context = PluginContext {
            id: package.manifest.id.clone(),
            manifest: Arc::new(package.manifest.clone()),
            locales: Arc::new(package.locales.clone()),
            language,
            day: None,
            log: Arc::new(Log { print: print_log }),
            clock: Arc::new(move || *now.lock().unwrap()),
        };
        let instance = runtime
            .instantiate(module, context)
            .map_err(|e| format!("the module did not start: {e}"))?;
        Ok(Run {
            package,
            language,
            instance,
            clock,
            card: None,
            events: 0,
            most_fuel: 0,
            longest: Duration::ZERO,
        })
    }

    fn report(&mut self) -> Result<String, String> {
        let manifest = &self.package.manifest;
        self.send(
            &Event::Installed {
                version: manifest.version.to_string(),
            },
            CallKind::Background,
        )?;
        self.send(
            &Event::Activate {
                language: self.language,
                reason: ActivateReason::First,
            },
            CallKind::Background,
        )?;
        let mut out = format!("# start\n{}", self.render());
        let listens = manifest
            .events
            .contains(&bientot_plugin_api::EventKind::DayRefreshed);
        for (name, mut day) in demo_days() {
            day.language = self.language;
            *self.clock.lock().unwrap() = day.computed_at;
            self.instance.state_mut().context.day = Some(
                serde_json::to_string(&day)
                    .expect("a day serializes")
                    .into(),
            );
            if listens {
                self.send(
                    &Event::DayRefreshed { day: Box::new(day) },
                    CallKind::Background,
                )
                .map_err(|e| format!("{name}: {e}"))?;
            }
            out.push_str(&format!("# {name}\n{}", self.render()));
        }
        for _ in 0..REPORT_PRESSES {
            let Some(button) = self
                .prepared()
                .and_then(|card| card.actions.first().cloned())
            else {
                break;
            };
            *self.clock.lock().unwrap() += REPORT_PRESS_STEP;
            self.send(
                &Event::CommandInvoked {
                    command: button.command.clone(),
                    arg: button.arg.clone(),
                    source: CommandSource::Card,
                },
                CallKind::User,
            )
            .map_err(|e| format!("press {}: {e}", button.label))?;
            out.push_str(&format!("# press {}\n{}", button.label, self.render()));
        }
        Ok(out)
    }

    fn send(&mut self, event: &Event, kind: CallKind) -> Result<(), String> {
        let bytes = serde_json::to_vec(event).expect("an event serializes");
        let outcome = self
            .instance
            .deliver(&bytes, Limits::for_event(kind, bytes.len()), &mut || {})
            .map_err(|e| format!("the plugin failed: {e}"))?;
        if let Some(refused) = outcome.rejected.first() {
            return Err(format!("Bientôt refused a card: {refused}"));
        }
        for change in outcome.ui {
            self.card = match change {
                UiChange::Set(card) => Some(card),
                UiChange::Clear => None,
            };
        }
        self.events += 1;
        self.most_fuel = self.most_fuel.max(outcome.fuel);
        self.longest = self.longest.max(outcome.busy);
        Ok(())
    }

    fn prepared(&self) -> Option<PreparedCard> {
        let context = &self.instance.state().context;
        let view = ViewContext {
            texts: context.texts(),
            manifest: &context.manifest,
        };
        self.card
            .as_ref()
            .map(|card| card.prepare(&view).expect("an accepted card prepares"))
    }

    fn render(&self) -> String {
        match self.prepared() {
            Some(card) => render_card(
                self.package.manifest.display_name(self.language),
                &card,
                *self.clock.lock().unwrap(),
            ),
            None => NO_CARD.to_owned(),
        }
    }
}
