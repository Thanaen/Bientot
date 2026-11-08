//! The `plugins` thread (FR-011, FR-012, FR-016, FR-017): a bounded queue in front of the
//! instances, user actions first, day snapshots coalesced, background events held to the CPU
//! budgets, idle instances evicted and bloated ones re-created.
//!
//! [`Host`] holds the logic and takes the time as an argument, so that tests drive it with a fake
//! sandbox and a fake clock; [`spawn`] runs it on its own thread.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bientot_plugin_api::event::{ActivateReason, CommandSource, Event};
use bientot_plugin_api::manifest::EventKind;
use bientot_plugin_api::snapshot::DaySnapshot;
use bientot_plugin_api::view::ViewContext;
use bientot_plugin_api::{Card, Language, LocaleTexts, Manifest, PluginId, PreparedCard};
use jiff::Timestamp;

use crate::budget::Budgets;
use crate::host::{Clock, LogSink, PluginContext, UiChange};
use crate::runtime::{CallError, CallKind, Limits, Outcome};
use crate::snapshot::is_structural;
use crate::state::Pending;

pub const QUEUE: usize = 64;
pub const IDLE_EVICTION: Duration = Duration::from_secs(5 * 60);
pub const DAY_INTERVAL: Duration = Duration::from_secs(30);
/// First activations at start are spread this far apart.
pub const ACTIVATION_STAGGER: Duration = Duration::from_millis(250);
pub const MIN_RECREATE_BYTES: usize = 1 << 20;
const IDLE_WAIT: Duration = Duration::from_secs(60);

/// A plugin the thread may run: checked on disk, turned on, its manifest read.
#[derive(Clone)]
pub struct PluginSpec {
    pub manifest: Arc<Manifest>,
    pub locales: Arc<std::collections::BTreeMap<String, LocaleTexts>>,
    pub wasm: Arc<Vec<u8>>,
    pub granted: BTreeSet<bientot_plugin_api::Capability>,
    /// `installed` or `upgraded` to deliver before the first `activate`.
    pub pending: Option<Pending>,
}

/// What runs plugin code: the wasmi runtime, or a fake in tests.
pub trait Sandbox {
    type Instance: Sandboxed;
    fn activate(
        &mut self,
        spec: &PluginSpec,
        context: PluginContext,
    ) -> Result<Self::Instance, CallError>;
    /// The plugin is gone: its compiled module can go too.
    fn forget(&mut self, id: &PluginId);
}

pub trait Sandboxed {
    fn deliver(
        &mut self,
        event: &[u8],
        limits: Limits,
        between: &mut dyn FnMut(),
    ) -> Result<Outcome, CallError>;
    fn memory_bytes(&self) -> usize;
    fn context_mut(&mut self) -> &mut PluginContext;
}

#[derive(Debug)]
pub enum Request {
    Load(Box<PluginSpec>),
    Unload(PluginId),
    Day(Arc<DaySnapshot>),
    Language(Language),
    Command {
        plugin: PluginId,
        command: String,
        arg: Option<String>,
        source: CommandSource,
    },
    Stop,
}

impl std::fmt::Debug for PluginSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginSpec")
            .field("id", &self.manifest.id)
            .finish_non_exhaustive()
    }
}

/// What the thread tells the app.
#[derive(Debug, Clone, PartialEq)]
pub enum HostUpdate {
    /// The plugin's card in the app language, or `None` to remove it.
    Card {
        plugin: PluginId,
        card: Option<PreparedCard>,
    },
    /// The plugin failed; the app records it and unloads the plugin after the third time in
    /// 10 minutes (FR-016).
    Failed {
        plugin: PluginId,
        error: String,
        at: Timestamp,
    },
    /// `installed` or `upgraded` was delivered.
    Delivered { plugin: PluginId },
}

/// The thread's records of the app's state, kept to answer `bt:day@1 current`.
struct Shared {
    language: Language,
    day: Option<Arc<DaySnapshot>>,
    day_json: Option<Arc<str>>,
}

struct Slot<I> {
    spec: PluginSpec,
    instance: Option<I>,
    reason: ActivateReason,
    size_after_activate: usize,
    last_event: Instant,
    /// Background events waiting, oldest first; the day is apart since only the latest counts.
    queued: VecDeque<Event>,
    day_due: bool,
    sent_day: Option<Arc<DaySnapshot>>,
    sent_day_at: Option<Instant>,
    card: Option<Card>,
    /// Not before this, because of the stagger at start.
    not_before: Instant,
}

impl<I> Slot<I> {
    fn wants(&self, kind: EventKind) -> bool {
        self.spec.manifest.events.contains(&kind)
    }
}

pub struct Host<S: Sandbox> {
    sandbox: S,
    slots: HashMap<PluginId, Slot<S::Instance>>,
    order: Vec<PluginId>,
    user: VecDeque<Request>,
    budgets: Budgets,
    shared: Shared,
    log: Arc<dyn LogSink>,
    clock: Clock,
    notify: Box<dyn FnMut(HostUpdate) + Send>,
    next_activation: Instant,
    /// Where requests that arrive during a call come from, read between two slices.
    inbox: Option<Box<dyn FnMut() -> Option<Request>>>,
    /// Requests read during a call that wait for it to end: anything but another plugin's
    /// command, which runs at once.
    later: VecDeque<Request>,
    /// The plugins whose instance is out of its slot for a call, outermost first.
    calling: Vec<PluginId>,
    stopped: bool,
}

impl<S: Sandbox> Host<S> {
    pub fn new(
        sandbox: S,
        language: Language,
        log: Arc<dyn LogSink>,
        clock: Clock,
        notify: Box<dyn FnMut(HostUpdate) + Send>,
        now: Instant,
    ) -> Host<S> {
        Host {
            sandbox,
            slots: HashMap::new(),
            order: Vec::new(),
            user: VecDeque::new(),
            budgets: Budgets::new(now),
            shared: Shared {
                language,
                day: None,
                day_json: None,
            },
            log,
            clock,
            notify,
            next_activation: now,
            inbox: None,
            later: VecDeque::new(),
            calling: Vec::new(),
            stopped: false,
        }
    }

    /// Lets the host read new requests between two slices of a long call, so that a user
    /// command for another plugin does not wait for it (FR-012).
    pub fn set_inbox(&mut self, inbox: Box<dyn FnMut() -> Option<Request>>) {
        self.inbox = Some(inbox);
    }

    /// Whether a [`Request::Stop`] was accepted.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn is_live(&self, id: &PluginId) -> bool {
        self.slots.get(id).is_some_and(|s| s.instance.is_some())
    }

    /// Takes one request. User commands wait in their own queue, which [`Host::step`] serves
    /// first.
    pub fn accept(&mut self, request: Request, now: Instant) {
        match request {
            Request::Load(spec) => self.load(*spec, now),
            Request::Unload(id) => self.unload(&id),
            Request::Day(day) => self.day(day),
            Request::Language(language) => self.language(language),
            command @ Request::Command { .. } => self.user.push_back(command),
            Request::Stop => self.stopped = true,
        }
    }

    fn load(&mut self, spec: PluginSpec, now: Instant) {
        let id = spec.manifest.id.clone();
        self.unload(&id);
        let mut queued = VecDeque::new();
        match &spec.pending {
            Some(Pending::Installed) => queued.push_back(Event::Installed {
                version: spec.manifest.version.to_string(),
            }),
            Some(Pending::Upgraded { from }) => queued.push_back(Event::Upgraded {
                from: from.clone(),
                version: spec.manifest.version.to_string(),
            }),
            None => {}
        }
        let not_before = self.next_activation.max(now);
        self.next_activation = not_before + ACTIVATION_STAGGER;
        let slot = Slot {
            day_due: self.shared.day.is_some()
                && spec.manifest.events.contains(&EventKind::DayRefreshed),
            spec,
            instance: None,
            reason: ActivateReason::First,
            size_after_activate: 0,
            last_event: now,
            queued,
            sent_day: None,
            sent_day_at: None,
            card: None,
            not_before,
        };
        self.slots.insert(id.clone(), slot);
        self.order.push(id);
    }

    fn unload(&mut self, id: &PluginId) {
        if self.slots.remove(id).is_some() {
            self.order.retain(|o| o != id);
            self.sandbox.forget(id);
            self.budgets.forget(id);
            (self.notify)(HostUpdate::Card {
                plugin: id.clone(),
                card: None,
            });
        }
    }

    fn day(&mut self, day: Arc<DaySnapshot>) {
        let json: Arc<str> = serde_json::to_string(&*day)
            .expect("a snapshot serializes")
            .into();
        for slot in self.slots.values_mut() {
            if let Some(instance) = &mut slot.instance {
                instance.context_mut().day = Some(json.clone());
            }
            if slot.wants(EventKind::DayRefreshed)
                && slot
                    .sent_day
                    .as_ref()
                    .is_none_or(|sent| is_structural(sent, &day))
            {
                slot.day_due = true;
            }
        }
        self.shared.day = Some(day);
        self.shared.day_json = Some(json);
    }

    fn language(&mut self, language: Language) {
        if language == self.shared.language {
            return;
        }
        self.shared.language = language;
        let ids = self.order.clone();
        for id in ids {
            let slot = self.slots.get_mut(&id).expect("ordered slots exist");
            if let Some(instance) = &mut slot.instance {
                instance.context_mut().language = language;
            }
            if slot.wants(EventKind::LanguageChanged) {
                slot.queued.push_back(Event::LanguageChanged { language });
            }
            // The host re-resolves the texts of the card it holds, whether or not the plugin
            // listens.
            let card = slot
                .card
                .as_ref()
                .and_then(|card| prepare(card, &slot.spec, language));
            if slot.card.is_some() {
                (self.notify)(HostUpdate::Card { plugin: id, card });
            }
        }
    }

    /// Runs everything due at `now`: user commands first, then each plugin's background events
    /// when its budget and the 30 s day rule allow, then evictions.
    pub fn step(&mut self, now: Instant) {
        self.accept_later(now);
        while let Some(request) = self.user.pop_front() {
            self.command(request, now);
        }
        let ids = self.order.clone();
        for id in ids {
            self.background(&id, now);
            while let Some(request) = self.user.pop_front() {
                self.command(request, now);
            }
            self.accept_later(now);
        }
        self.evict_idle(now);
    }

    fn accept_later(&mut self, now: Instant) {
        while let Some(request) = self.later.pop_front() {
            self.accept(request, now);
        }
    }

    /// When [`Host::step`] has something to do again, if nothing new arrives.
    pub fn next_wakeup(&mut self, now: Instant) -> Instant {
        let mut next = now + IDLE_WAIT;
        let ids = self.order.clone();
        for id in ids {
            let ready = self.budgets.background_ready_at(&id, now);
            let slot = &self.slots[&id];
            if slot.instance.is_some() {
                next = next.min(slot.last_event + IDLE_EVICTION);
            }
            if slot.day_due || !slot.queued.is_empty() {
                let day_ready = slot
                    .sent_day_at
                    .filter(|_| slot.queued.is_empty())
                    .map_or(now, |at| at + DAY_INTERVAL);
                next = next.min(ready.max(day_ready).max(slot.not_before));
            }
        }
        next.max(now)
    }

    fn command(&mut self, request: Request, now: Instant) {
        let Request::Command {
            plugin,
            command,
            arg,
            source,
        } = request
        else {
            return;
        };
        let known = self
            .slots
            .get(&plugin)
            .is_some_and(|slot| slot.spec.manifest.command(&command).is_some());
        if !known {
            return;
        }
        let event = Event::CommandInvoked {
            command,
            arg,
            source,
        };
        // A user action does not wait for the stagger or the budget, but a pending `installed`
        // still comes first.
        self.flush_queued(&plugin, now);
        self.deliver(&plugin, &event, CallKind::User, now);
    }

    fn flush_queued(&mut self, id: &PluginId, now: Instant) {
        if !self.ensure_instance(id, now) {
            return;
        }
        while let Some(event) = self.slots.get_mut(id).and_then(|s| s.queued.pop_front()) {
            if !self.deliver(id, &event, CallKind::Background, now) {
                break;
            }
        }
    }

    fn background(&mut self, id: &PluginId, now: Instant) {
        let Some(slot) = self.slots.get(id) else {
            return;
        };
        if now < slot.not_before || self.budgets.background_ready_at(id, now) > now {
            return;
        }
        let day_allowed = slot.sent_day_at.is_none_or(|at| now >= at + DAY_INTERVAL);
        let day_ready = slot.day_due && day_allowed && self.shared.day.is_some();
        if slot.queued.is_empty() && !day_ready {
            return;
        }
        if !self.ensure_instance(id, now) {
            return;
        }
        if let Some(event) = self.slots.get_mut(id).and_then(|s| s.queued.pop_front()) {
            self.deliver(id, &event, CallKind::Background, now);
            return;
        }
        let (Some(day), Some(slot)) = (self.shared.day.clone(), self.slots.get_mut(id)) else {
            return;
        };
        if !slot.day_due {
            return;
        }
        slot.day_due = false;
        slot.sent_day = Some(day.clone());
        slot.sent_day_at = Some(now);
        let event = Event::DayRefreshed {
            day: Box::new((*day).clone()),
        };
        self.deliver(id, &event, CallKind::Background, now);
    }

    /// Delivers `event` to `id`, activating it first if needed. Returns whether it ran.
    fn deliver(&mut self, id: &PluginId, event: &Event, kind: CallKind, now: Instant) -> bool {
        if !self.ensure_instance(id, now) {
            return false;
        }
        let bytes = serde_json::to_vec(event).expect("an event serializes");
        let mut result = self.call(id, &bytes, kind, now);
        if result == Err(CallError::PoolFull) {
            self.evict_others(id);
            if self.ensure_instance(id, now) {
                result = self.call(id, &bytes, kind, now);
            }
        }
        if matches!(event, Event::Installed { .. } | Event::Upgraded { .. }) && result.is_ok() {
            (self.notify)(HostUpdate::Delivered { plugin: id.clone() });
        }
        result.is_ok()
    }

    fn call(
        &mut self,
        id: &PluginId,
        bytes: &[u8],
        kind: CallKind,
        now: Instant,
    ) -> Result<(), CallError> {
        let Some(mut instance) = self.slots.get_mut(id).and_then(|s| s.instance.take()) else {
            return Err(CallError::Trap("no instance".into()));
        };
        let started = Instant::now();
        let limits = Limits::for_event(kind, bytes.len());
        self.calling.push(id.clone());
        let result = {
            let mut between = || self.serve_others(now);
            instance.deliver(bytes, limits, &mut between)
        };
        self.calling.pop();
        let busy = result
            .as_ref()
            .map_or_else(|_| started.elapsed(), |o| o.busy);
        self.budgets.charge(id, busy, now);
        match result {
            Ok(outcome) => {
                self.apply(id, outcome);
                let slot = self.slots.get_mut(id).expect("the slot outlives its call");
                slot.last_event = now;
                let size = instance.memory_bytes();
                if size > (2 * slot.size_after_activate).max(MIN_RECREATE_BYTES) {
                    slot.reason = ActivateReason::Recreated;
                } else {
                    slot.instance = Some(instance);
                }
                Ok(())
            }
            Err(error) => {
                self.failed(id, &error);
                Err(error)
            }
        }
    }

    /// Between two slices of a long call: read what arrived and run user commands of the other
    /// plugins. The rest waits for the call to end: an `Unload` of the running plugin cannot
    /// take its slot away under it.
    fn serve_others(&mut self, now: Instant) {
        if let Some(inbox) = self.inbox.as_mut() {
            let mut arrived = Vec::new();
            while let Some(request) = inbox() {
                arrived.push(request);
            }
            for request in arrived {
                match request {
                    command @ Request::Command { .. } => self.user.push_back(command),
                    other => self.later.push_back(other),
                }
            }
        }
        let mut kept = VecDeque::new();
        while let Some(request) = self.user.pop_front() {
            match &request {
                // A plugin in a call, this one or one that called into a command, waits.
                Request::Command { plugin, .. } if !self.calling.contains(plugin) => {
                    self.command(request, now)
                }
                _ => kept.push_back(request),
            }
        }
        self.user = kept;
    }

    fn ensure_instance(&mut self, id: &PluginId, now: Instant) -> bool {
        let Some(slot) = self.slots.get_mut(id) else {
            return false;
        };
        if slot.instance.is_some() {
            return true;
        }
        let context = PluginContext {
            id: id.clone(),
            manifest: slot.spec.manifest.clone(),
            locales: slot.spec.locales.clone(),
            language: self.shared.language,
            day: self.shared.day_json.clone(),
            log: self.log.clone(),
            clock: self.clock.clone(),
        };
        let started = Instant::now();
        let mut activated = self.sandbox.activate(&slot.spec, context);
        if activated.as_ref().err() == Some(&CallError::PoolFull) {
            self.evict_others(id);
            let slot = &self.slots[id];
            let context = PluginContext {
                id: id.clone(),
                manifest: slot.spec.manifest.clone(),
                locales: slot.spec.locales.clone(),
                language: self.shared.language,
                day: self.shared.day_json.clone(),
                log: self.log.clone(),
                clock: self.clock.clone(),
            };
            activated = self.sandbox.activate(&slot.spec, context);
        }
        // The start function is the plugin's code too: its time counts, failed or not.
        self.budgets.charge(id, started.elapsed(), now);
        let instance = match activated {
            Ok(instance) => instance,
            Err(error) => {
                self.failed(id, &error);
                return false;
            }
        };
        let slot = self.slots.get_mut(id).expect("checked above");
        slot.instance = Some(instance);
        // `installed` or `upgraded` goes before the first `activate`.
        let pending: Vec<Event> = slot
            .queued
            .iter()
            .filter(|e| matches!(e, Event::Installed { .. } | Event::Upgraded { .. }))
            .cloned()
            .collect();
        slot.queued
            .retain(|e| !matches!(e, Event::Installed { .. } | Event::Upgraded { .. }));
        for event in pending {
            if !self.deliver_now(id, &event, now) {
                return false;
            }
            (self.notify)(HostUpdate::Delivered { plugin: id.clone() });
        }
        let reason = self.slots[id].reason;
        let activate = Event::Activate {
            language: self.shared.language,
            reason,
        };
        if !self.deliver_now(id, &activate, now) {
            return false;
        }
        let slot = self.slots.get_mut(id).expect("checked above");
        slot.size_after_activate = slot.instance.as_ref().map_or(0, Sandboxed::memory_bytes);
        slot.reason = ActivateReason::First;
        slot.instance.is_some()
    }

    fn deliver_now(&mut self, id: &PluginId, event: &Event, now: Instant) -> bool {
        let bytes = serde_json::to_vec(event).expect("an event serializes");
        self.call(id, &bytes, CallKind::Background, now).is_ok()
    }

    fn apply(&mut self, id: &PluginId, outcome: Outcome) {
        let language = self.shared.language;
        let slot = self.slots.get_mut(id).expect("applied to a live slot");
        for rejected in &outcome.rejected {
            self.log.host(id, &format!("card refused: {rejected}"));
        }
        for change in outcome.ui {
            let (card, prepared) = match change {
                UiChange::Set(card) => {
                    let prepared = prepare(&card, &slot.spec, language);
                    (Some(card), prepared)
                }
                UiChange::Clear => (None, None),
            };
            slot.card = card;
            (self.notify)(HostUpdate::Card {
                plugin: id.clone(),
                card: prepared,
            });
        }
        if !outcome.rejected.is_empty() {
            (self.notify)(HostUpdate::Failed {
                plugin: id.clone(),
                error: outcome.rejected.join("; "),
                at: (self.clock)(),
            });
        }
    }

    fn failed(&mut self, id: &PluginId, error: &CallError) {
        self.log.host(id, &error.to_string());
        if let Some(slot) = self.slots.get_mut(id) {
            slot.instance = None;
            slot.reason = ActivateReason::Restart;
        }
        if error.is_failure() {
            (self.notify)(HostUpdate::Failed {
                plugin: id.clone(),
                error: error.to_string(),
                at: (self.clock)(),
            });
        }
    }

    fn evict_idle(&mut self, now: Instant) {
        for slot in self.slots.values_mut() {
            if slot.instance.is_some() && now >= slot.last_event + IDLE_EVICTION {
                slot.instance = None;
                slot.reason = ActivateReason::Evicted;
            }
        }
    }

    fn evict_others(&mut self, keep: &PluginId) {
        for (id, slot) in &mut self.slots {
            if id != keep && slot.instance.is_some() {
                slot.instance = None;
                slot.reason = ActivateReason::Evicted;
            }
        }
    }
}

fn prepare(card: &Card, spec: &PluginSpec, language: Language) -> Option<PreparedCard> {
    let fallback = &spec.locales[spec.manifest.fallback_locale()];
    let context = ViewContext {
        texts: bientot_plugin_api::locale::TextLookup {
            preferred: spec.locales.get(language.tag()),
            fallback,
        },
        manifest: &spec.manifest,
    };
    card.prepare(&context).ok()
}

/// The handle the app keeps. Dropping it stops the thread.
pub struct PluginThread {
    /// `None` once dropping: the thread then sees the channel close even if `Stop` did not fit.
    sender: Option<SyncSender<Request>>,
    handle: Option<JoinHandle<()>>,
    shared_full: Arc<Mutex<u64>>,
}

impl PluginThread {
    /// Sends a request. A full queue drops it, whatever it is, and counts the drop for
    /// diagnostics: 64 requests waiting means the thread is stuck, and a dropped day snapshot is
    /// replaced by the next one.
    pub fn send(&self, request: Request) {
        let Some(sender) = &self.sender else { return };
        if let Err(TrySendError::Full(_)) = sender.try_send(request) {
            *self.shared_full.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
    }

    pub fn dropped(&self) -> u64 {
        *self.shared_full.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for PluginThread {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.try_send(Request::Stop);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Starts the `plugins` thread. `make` builds the sandbox on that thread, so that no engine
/// exists before (FR-018).
pub fn spawn<S, F>(
    make: F,
    language: Language,
    log: Arc<dyn LogSink>,
    clock: Clock,
    notify: Box<dyn FnMut(HostUpdate) + Send>,
) -> std::io::Result<PluginThread>
where
    S: Sandbox + 'static,
    F: FnOnce() -> S + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(QUEUE);
    let handle = std::thread::Builder::new()
        .name("plugins".into())
        .stack_size(8 << 20)
        .spawn(move || run(make(), receiver, language, log, clock, notify))?;
    Ok(PluginThread {
        sender: Some(sender),
        handle: Some(handle),
        shared_full: Arc::default(),
    })
}

fn run<S: Sandbox>(
    sandbox: S,
    receiver: Receiver<Request>,
    language: Language,
    log: Arc<dyn LogSink>,
    clock: Clock,
    notify: Box<dyn FnMut(HostUpdate) + Send>,
) {
    let receiver = std::rc::Rc::new(receiver);
    let mut host = Host::new(sandbox, language, log, clock, notify, Instant::now());
    let inbox = receiver.clone();
    host.set_inbox(Box::new(move || inbox.try_recv().ok()));
    loop {
        let now = Instant::now();
        let wait = host.next_wakeup(now).saturating_duration_since(now);
        match receiver.recv_timeout(wait) {
            Ok(Request::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(request) => host.accept(request, Instant::now()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        // Take what else is waiting, so that the latest day wins and commands go first.
        while let Ok(request) = receiver.try_recv() {
            if matches!(request, Request::Stop) {
                return;
            }
            host.accept(request, Instant::now());
        }
        host.step(Instant::now());
        if host.stopped() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
