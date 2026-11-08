use std::collections::BTreeMap;

use bientot_plugin_api::{Capability, view::Prepared};
use serde_json::{Value, json};

use super::*;
use crate::runtime::tests::Lines;

/// What the fake instances saw and how they behave.
#[derive(Default)]
struct Script {
    /// (plugin, event name, reason or command), in order.
    seen: Vec<(String, String, String)>,
    activations: u32,
    /// Events whose name is here trap.
    trap_on: Vec<String>,
    busy: Duration,
    /// Memory after each event.
    memory: usize,
    /// Set a card on `activate`.
    card_on_activate: bool,
    /// Requests that reach `inbox` while `bientot.a` runs, before it calls `between`.
    arrive: Vec<Request>,
    inbox: Option<Arc<Mutex<VecDeque<Request>>>>,
    /// Time an activation takes.
    activation: Duration,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Script>>);

struct FakeInstance {
    id: PluginId,
    script: Arc<Mutex<Script>>,
    context: PluginContext,
}

impl Sandbox for Fake {
    type Instance = FakeInstance;

    fn activate(
        &mut self,
        spec: &PluginSpec,
        context: PluginContext,
    ) -> Result<FakeInstance, CallError> {
        let activation = {
            let mut script = self.0.lock().unwrap();
            script.activations += 1;
            script.activation
        };
        std::thread::sleep(activation);
        Ok(FakeInstance {
            id: spec.manifest.id.clone(),
            script: self.0.clone(),
            context,
        })
    }

    fn forget(&mut self, _: &PluginId) {}
}

impl Sandboxed for FakeInstance {
    fn deliver(
        &mut self,
        event: &[u8],
        _: Limits,
        between: &mut dyn FnMut(),
    ) -> Result<Outcome, CallError> {
        {
            let mut script = self.script.lock().unwrap();
            if self.id.as_str() == "bientot.a"
                && let Some(inbox) = script.inbox.clone()
            {
                inbox.lock().unwrap().extend(script.arrive.drain(..));
            }
        }
        between();
        let event: Value = serde_json::from_slice(event).unwrap();
        let name = event["event"].as_str().unwrap().to_owned();
        let detail = ["reason", "command", "language"]
            .iter()
            .find_map(|k| event[k].as_str())
            .unwrap_or("")
            .to_owned();
        let mut script = self.script.lock().unwrap();
        script
            .seen
            .push((self.id.as_str().to_owned(), name.clone(), detail));
        if script.trap_on.contains(&name) {
            return Err(CallError::Trap("unreachable".into()));
        }
        let mut ui = Vec::new();
        if name == "activate" && script.card_on_activate {
            let card = json!({ "type": "card", "lines": [{ "type": "text", "key": "title" }] });
            ui.push(UiChange::Set(Card::from_value(card).unwrap()));
        }
        Ok(Outcome {
            ui,
            rejected: Vec::new(),
            fuel: 1,
            busy: script.busy,
        })
    }

    fn memory_bytes(&self) -> usize {
        self.script.lock().unwrap().memory
    }

    fn context_mut(&mut self) -> &mut PluginContext {
        &mut self.context
    }
}

fn spec(id: &str, events: &[&str], pending: Option<Pending>) -> PluginSpec {
    let manifest = json!({
        "id": id,
        "name": { "en": "Test" },
        "version": "1.2.0",
        "api": "1.0",
        "locales": ["en", "fr"],
        "capabilities": ["read:day", "ui:flyout"],
        "events": events,
        "commands": [{ "name": "go", "title": { "en": "Go" } }]
    });
    let locales: BTreeMap<String, LocaleTexts> = [
        (
            "en".to_owned(),
            LocaleTexts::from_pairs([("title", "Hello")]),
        ),
        (
            "fr".to_owned(),
            LocaleTexts::from_pairs([("title", "Bonjour")]),
        ),
    ]
    .into_iter()
    .collect();
    PluginSpec {
        manifest: Arc::new(Manifest::parse(manifest.to_string().as_bytes(), &[]).unwrap()),
        locales: Arc::new(locales),
        wasm: Arc::new(Vec::new()),
        granted: BTreeSet::from([Capability::ReadDay, Capability::UiFlyout]),
        pending,
    }
}

struct Harness {
    host: Host<Fake>,
    fake: Fake,
    updates: Arc<Mutex<Vec<HostUpdate>>>,
    t0: Instant,
}

impl Harness {
    fn new() -> Harness {
        let fake = Fake::default();
        let updates: Arc<Mutex<Vec<HostUpdate>>> = Arc::default();
        let sink = updates.clone();
        let t0 = Instant::now();
        let host = Host::new(
            fake.clone(),
            Language::En,
            Arc::new(Lines::default()),
            Arc::new(|| "2026-11-08T12:00:00Z".parse().unwrap()),
            Box::new(move |update| sink.lock().unwrap().push(update)),
            t0,
        );
        Harness {
            host,
            fake,
            updates,
            t0,
        }
    }

    fn at(&self, seconds: f64) -> Instant {
        self.t0 + Duration::from_secs_f64(seconds)
    }

    fn send(&mut self, request: Request, seconds: f64) {
        let now = self.at(seconds);
        self.host.accept(request, now);
        self.host.step(now);
    }

    fn seen(&self) -> Vec<(String, String, String)> {
        std::mem::take(&mut self.fake.0.lock().unwrap().seen)
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.fake.0.lock().unwrap()
    }
}

fn day(minutes: i64) -> Arc<DaySnapshot> {
    let mut day: DaySnapshot = serde_json::from_str(include_str!(
        "../../../bientot-plugin-api/tests/fixtures/api-1.0/day-snapshot.json"
    ))
    .unwrap();
    day.departure_at_target = day
        .departure_at_target
        .map(|t| t + jiff::SignedDuration::from_mins(minutes));
    Arc::new(day)
}

fn seen(entries: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    entries
        .iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect()
}

fn command(plugin: &str) -> Request {
    Request::Command {
        plugin: PluginId::parse(plugin).unwrap(),
        command: "go".into(),
        arg: None,
        source: CommandSource::Palette,
    }
}

#[test]
fn installed_comes_before_activate_then_the_day() {
    let mut h = Harness::new();
    h.send(Request::Day(day(0)), 0.0);
    h.send(
        Request::Load(Box::new(spec(
            "bientot.a",
            &["day.refreshed"],
            Some(Pending::Installed),
        ))),
        0.0,
    );
    assert_eq!(
        h.seen(),
        seen(&[
            ("bientot.a", "installed", ""),
            ("bientot.a", "activate", "first"),
            ("bientot.a", "day.refreshed", ""),
        ])
    );
    assert!(h.updates.lock().unwrap().contains(&HostUpdate::Delivered {
        plugin: PluginId::parse("bientot.a").unwrap()
    }));
}

#[test]
fn nothing_runs_until_the_first_event() {
    let mut h = Harness::new();
    h.send(Request::Load(Box::new(spec("bientot.a", &[], None))), 0.0);
    assert_eq!(h.script().activations, 0);
    h.send(command("bientot.a"), 1.0);
    assert_eq!(
        h.seen(),
        seen(&[
            ("bientot.a", "activate", "first"),
            ("bientot.a", "command.invoked", "go")
        ])
    );
}

#[test]
fn day_snapshots_are_coalesced_and_sent_at_most_every_30_seconds() {
    let mut h = Harness::new();
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        0.0,
    );
    h.send(Request::Day(day(0)), 1.0);
    assert_eq!(h.seen().len(), 2);
    // Not structural: nothing.
    h.send(Request::Day(day(0)), 40.0);
    assert!(h.seen().is_empty());
    // Two structural ones within 30 s of the last: only the latest, at 31 s.
    h.send(Request::Day(day(5)), 10.0);
    h.send(Request::Day(day(6)), 20.0);
    assert!(h.seen().is_empty());
    let next = h.host.next_wakeup(h.at(20.0));
    assert_eq!(next, h.at(31.0));
    h.host.step(next);
    assert_eq!(h.seen(), seen(&[("bientot.a", "day.refreshed", "")]));
}

#[test]
fn a_plugin_that_does_not_listen_gets_no_day() {
    let mut h = Harness::new();
    h.send(Request::Load(Box::new(spec("bientot.a", &[], None))), 0.0);
    h.send(Request::Day(day(0)), 1.0);
    assert!(h.seen().is_empty());
}

#[test]
fn user_commands_go_first_and_ignore_the_budget() {
    let mut h = Harness::new();
    // `activate` and the first day take 600 ms: 100 ms past the 500 ms reserve, which 20 ms a
    // minute pays back in 5 minutes.
    h.script().busy = Duration::from_millis(300);
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        0.0,
    );
    h.send(Request::Day(day(0)), 1.0);
    h.seen();
    h.script().busy = Duration::ZERO;
    // In debt, a structural day waits, a command does not.
    h.host.accept(Request::Day(day(5)), h.at(40.0));
    h.host.accept(command("bientot.a"), h.at(40.0));
    h.host.step(h.at(40.0));
    assert_eq!(h.seen(), seen(&[("bientot.a", "command.invoked", "go")]));
    h.host.step(h.at(250.0));
    assert!(h.seen().is_empty());
    h.host.step(h.at(302.0));
    assert_eq!(h.seen(), seen(&[("bientot.a", "day.refreshed", "")]));
}

#[test]
fn a_long_call_lets_other_plugins_commands_through() {
    let mut h = Harness::new();
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        0.0,
    );
    h.send(Request::Load(Box::new(spec("bientot.b", &[], None))), 0.0);
    h.seen();
    // b's command waits in the queue while a runs; the fake calls `between` at once.
    h.host.accept(Request::Day(day(0)), h.at(1.0));
    h.host.accept(command("bientot.b"), h.at(1.0));
    h.host.step(h.at(1.0));
    let order: Vec<String> = h
        .seen()
        .into_iter()
        .map(|(p, e, _)| format!("{p} {e}"))
        .collect();
    assert_eq!(order[0], "bientot.b activate", "{order:?}");
}

#[test]
fn a_command_that_arrives_during_a_long_call_runs_between_slices() {
    let mut h = Harness::new();
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        0.0,
    );
    h.send(Request::Load(Box::new(spec("bientot.b", &[], None))), 0.0);
    h.seen();
    let inbox: Arc<Mutex<VecDeque<Request>>> = Arc::default();
    let reader = inbox.clone();
    h.host
        .set_inbox(Box::new(move || reader.lock().unwrap().pop_front()));
    {
        let mut script = h.script();
        script.inbox = Some(inbox);
        let a = PluginId::parse("bientot.a").unwrap();
        // b's command and a's own command and unload arrive while a's day call runs.
        script.arrive = vec![
            command("bientot.b"),
            command("bientot.a"),
            Request::Unload(a),
        ];
    }
    h.host.accept(Request::Day(day(0)), h.at(1.0));
    h.host.step(h.at(1.0));
    let order: Vec<String> = h
        .seen()
        .into_iter()
        .map(|(p, e, _)| format!("{p} {e}"))
        .collect();
    assert_eq!(
        order,
        [
            "bientot.b activate",
            "bientot.b command.invoked",
            "bientot.a activate",
            "bientot.a day.refreshed",
            "bientot.a command.invoked",
        ],
        "b ran between a's slices; a's own command and its unload waited for the call to end"
    );
    assert!(!h.host.is_live(&PluginId::parse("bientot.a").unwrap()));
}

#[test]
fn an_activation_is_charged_to_the_budget() {
    let mut h = Harness::new();
    h.script().activation = crate::budget::PLUGIN_RESERVE + Duration::from_millis(20);
    h.host.accept(Request::Day(day(0)), h.at(0.0));
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        0.0,
    );
    let a = PluginId::parse("bientot.a").unwrap();
    assert!(h.host.budgets.background_ready_at(&a, h.at(0.0)) > h.at(0.0));
}

#[test]
fn the_thread_stops_on_drop_even_with_a_full_queue() {
    let thread = spawn(
        || {
            // The thread starts late, so the queue is full when the handle drops.
            std::thread::sleep(Duration::from_millis(200));
            Fake::default()
        },
        Language::En,
        Arc::new(Lines::default()),
        Arc::new(|| "2026-11-08T12:00:00Z".parse().unwrap()),
        Box::new(|_| {}),
    )
    .unwrap();
    for n in 0..(2 * QUEUE) {
        thread.send(Request::Load(Box::new(spec(
            &format!("bientot.p{n}"),
            &[],
            None,
        ))));
    }
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(thread);
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("the handle stops the thread");
}

#[test]
fn first_activations_are_staggered() {
    let mut h = Harness::new();
    h.host.accept(Request::Day(day(0)), h.at(0.0));
    h.host.accept(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        h.at(0.0),
    );
    h.host.accept(
        Request::Load(Box::new(spec("bientot.b", &["day.refreshed"], None))),
        h.at(0.0),
    );
    h.host.step(h.at(0.0));
    assert_eq!(h.script().activations, 1);
    assert_eq!(h.host.next_wakeup(h.at(0.0)), h.at(0.25));
    h.host.step(h.at(0.25));
    assert_eq!(h.script().activations, 2);
}

#[test]
fn idle_instances_are_evicted_and_come_back_with_the_reason() {
    let mut h = Harness::new();
    h.send(Request::Load(Box::new(spec("bientot.a", &[], None))), 0.0);
    h.send(command("bientot.a"), 1.0);
    let id = PluginId::parse("bientot.a").unwrap();
    assert!(h.host.is_live(&id));
    // Nothing to do before the eviction but the idle wake-up a minute later.
    assert_eq!(h.host.next_wakeup(h.at(2.0)), h.at(62.0));
    h.host.step(h.at(302.0));
    assert!(!h.host.is_live(&id));
    h.seen();
    h.send(command("bientot.a"), 303.0);
    assert_eq!(
        h.seen()[0],
        ("bientot.a".into(), "activate".into(), "evicted".into())
    );
}

#[test]
fn a_bloated_instance_is_recreated_between_events() {
    let mut h = Harness::new();
    h.send(Request::Load(Box::new(spec("bientot.a", &[], None))), 0.0);
    h.script().memory = 1 << 20;
    h.send(command("bientot.a"), 1.0);
    h.script().memory = 3 << 20;
    h.send(command("bientot.a"), 2.0);
    assert!(!h.host.is_live(&PluginId::parse("bientot.a").unwrap()));
    h.seen();
    h.send(command("bientot.a"), 3.0);
    assert_eq!(h.seen()[0].2, "recreated");
}

#[test]
fn a_trap_drops_the_instance_and_is_reported() {
    let mut h = Harness::new();
    h.send(Request::Load(Box::new(spec("bientot.a", &[], None))), 0.0);
    h.script().trap_on = vec!["command.invoked".into()];
    h.send(command("bientot.a"), 1.0);
    let id = PluginId::parse("bientot.a").unwrap();
    assert!(!h.host.is_live(&id));
    assert!(
        h.updates
            .lock()
            .unwrap()
            .iter()
            .any(|u| matches!(u, HostUpdate::Failed { plugin, .. } if *plugin == id))
    );
    h.script().trap_on.clear();
    h.seen();
    h.send(command("bientot.a"), 2.0);
    assert_eq!(h.seen()[0].2, "restart");
}

#[test]
fn a_language_change_redraws_the_card_and_tells_listeners() {
    let mut h = Harness::new();
    h.script().card_on_activate = true;
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["language.changed"], None))),
        0.0,
    );
    h.send(command("bientot.a"), 1.0);
    let text = |update: &HostUpdate| match update {
        HostUpdate::Card {
            card: Some(card), ..
        } => match &card.lines[0] {
            Prepared::Text { text, .. } => Some(text.clone()),
            _ => None,
        },
        _ => None,
    };
    let last = || h.updates.lock().unwrap().iter().rev().find_map(text);
    assert_eq!(last(), Some("Hello".into()));
    h.seen();
    h.send(Request::Language(Language::Fr), 2.0);
    assert_eq!(
        h.updates.lock().unwrap().iter().rev().find_map(text),
        Some("Bonjour".into())
    );
    assert_eq!(h.seen(), seen(&[("bientot.a", "language.changed", "fr")]));
}

#[test]
fn unloading_removes_the_card() {
    let mut h = Harness::new();
    h.send(Request::Load(Box::new(spec("bientot.a", &[], None))), 0.0);
    h.send(Request::Unload(PluginId::parse("bientot.a").unwrap()), 1.0);
    assert!(h.host.is_empty());
    assert!(matches!(
        h.updates.lock().unwrap().last(),
        Some(HostUpdate::Card { card: None, .. })
    ));
    // A command for a plugin that is gone is dropped.
    h.send(command("bientot.a"), 2.0);
    assert!(h.seen().is_empty());
}

#[test]
fn the_thread_runs_requests_and_stops_on_drop() {
    let updates: Arc<Mutex<Vec<HostUpdate>>> = Arc::default();
    let sink = updates.clone();
    let fake = Fake::default();
    fake.0.lock().unwrap().card_on_activate = true;
    let thread = spawn(
        move || fake,
        Language::En,
        Arc::new(Lines::default()),
        Arc::new(Timestamp::now),
        Box::new(move |update| sink.lock().unwrap().push(update)),
    )
    .unwrap();
    thread.send(Request::Load(Box::new(spec("bientot.a", &[], None))));
    thread.send(command("bientot.a"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while updates.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(thread);
    assert!(matches!(
        updates.lock().unwrap()[0],
        HostUpdate::Card { card: Some(_), .. }
    ));
}

#[test]
fn after_a_failure_background_events_wait_but_commands_do_not() {
    let mut h = Harness::new();
    h.script().trap_on = vec!["activate".into()];
    h.send(Request::Day(day(0)), 0.0);
    h.send(
        Request::Load(Box::new(spec("bientot.a", &["day.refreshed"], None))),
        0.0,
    );
    let failures = |h: &Harness| {
        h.updates
            .lock()
            .unwrap()
            .iter()
            .filter(|u| matches!(u, HostUpdate::Failed { .. }))
            .count()
    };
    for t in [0.001, 0.01, 1.0, 29.0] {
        h.host.step(h.at(t));
    }
    assert_eq!(failures(&h), 1, "the day is not retried at once");
    assert_eq!(h.host.next_wakeup(h.at(1.0)), h.at(30.0));
    h.send(command("bientot.a"), 2.0);
    assert_eq!(failures(&h), 2, "a command still runs");
    h.host.step(h.at(32.1));
    assert_eq!(failures(&h), 3);
}
