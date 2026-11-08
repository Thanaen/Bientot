//! The host side of the imports (contract `host-abi.md`, « Imports of this step »): `bt:log@1`,
//! `bt:time@1`, `bt:day@1` and `bt:ui@1`. Each takes a JSON request at `(ptr, len)` and answers
//! with a buffer it allocates through `bt_alloc`, or `0`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use bientot_plugin_api::locale::TextLookup;
use bientot_plugin_api::view::ViewContext;
use bientot_plugin_api::{Capability, Card, Language, LocaleTexts, Manifest, PluginId};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use wasmi::{Caller, Error, Extern, FuncType, Linker, ValType};

use crate::runtime::Limiter;

pub const MAX_REQUEST_BYTES: usize = 64 << 10;
pub const MAX_REQUEST_DEPTH: usize = 32;
pub const IMPORT_FUEL: u64 = 10_000;
pub const FUEL_PER_BYTE: u64 = 250;
pub const MAX_UI_SETS: u32 = 2;
pub const MAX_LOG_CHARS: usize = 2_048;
pub const LOG_LINES_PER_MINUTE: u32 = 60;
pub const CARD_SLOT: &str = "flyout.card";
/// Fuel lent to a `bt_alloc` the host calls from inside an import; what it uses is charged.
const NESTED_FUEL: u64 = 1_000_000;

/// Where plugin log lines go. The file writer is [`crate::log::LogFiles`].
pub trait LogSink: Send + Sync {
    fn write(&self, plugin: &PluginId, level: LogLevel, text: &str);
    fn dropped(&self, plugin: &PluginId, count: u32);
    /// A line the host writes about the plugin: a trap, a refused card.
    fn host(&self, plugin: &PluginId, text: &str);
}

/// One plugin's log, held to [`LOG_LINES_PER_MINUTE`]. It outlives the plugin's instances, so
/// that a plugin that traps or is re-created gets no fresh minute, and it reports the lines it
/// dropped when the next minute starts and when it goes, so that a plugin that floods and then
/// falls silent still has its count in the log.
pub struct LimitedLog {
    inner: Arc<dyn LogSink>,
    window: std::sync::Mutex<LogWindow>,
}

#[derive(Default)]
struct LogWindow {
    plugin: Option<PluginId>,
    start: Option<Instant>,
    lines: u32,
    dropped: u32,
}

impl LogWindow {
    fn report(&mut self, inner: &dyn LogSink) {
        if let Some(plugin) = &self.plugin
            && self.dropped > 0
        {
            inner.dropped(plugin, self.dropped);
        }
        self.dropped = 0;
    }
}

impl LimitedLog {
    pub fn new(inner: Arc<dyn LogSink>) -> LimitedLog {
        LimitedLog {
            inner,
            window: Default::default(),
        }
    }

    fn write_at(&self, plugin: &PluginId, level: LogLevel, text: &str, now: Instant) {
        let mut window = self.window.lock().unwrap_or_else(|e| e.into_inner());
        window.plugin.get_or_insert_with(|| plugin.clone());
        if window
            .start
            .is_none_or(|start| now.duration_since(start).as_secs() >= 60)
        {
            window.report(&*self.inner);
            window.start = Some(now);
            window.lines = 0;
        }
        if window.lines >= LOG_LINES_PER_MINUTE {
            window.dropped += 1;
            return;
        }
        window.lines += 1;
        drop(window);
        self.inner.write(plugin, level, text);
    }
}

impl LogSink for LimitedLog {
    fn write(&self, plugin: &PluginId, level: LogLevel, text: &str) {
        self.write_at(plugin, level, text, Instant::now());
    }

    fn dropped(&self, plugin: &PluginId, count: u32) {
        self.inner.dropped(plugin, count);
    }

    fn host(&self, plugin: &PluginId, text: &str) {
        self.inner.host(plugin, text);
    }
}

impl Drop for LimitedLog {
    fn drop(&mut self) {
        let window = self.window.get_mut().unwrap_or_else(|e| e.into_inner());
        window.report(&*self.inner);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Everything an instance's imports read.
#[derive(Clone)]
pub struct PluginContext {
    pub id: PluginId,
    pub manifest: Arc<Manifest>,
    pub locales: Arc<BTreeMap<String, LocaleTexts>>,
    pub language: Language,
    /// The current day snapshot as JSON, or `None` before the first refresh.
    pub day: Option<Arc<str>>,
    pub log: Arc<dyn LogSink>,
    pub clock: Clock,
}

impl PluginContext {
    pub fn texts(&self) -> TextLookup<'_> {
        TextLookup {
            preferred: self.locales.get(self.language.tag()),
            fallback: &self.locales[self.manifest.fallback_locale()],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiChange {
    /// A tree that passed validation for the language of the call.
    Set(Card),
    Clear,
}

/// An import's reason to stop the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    OutOfFuel,
    BadRequest(String),
    Failed(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::OutOfFuel => write!(f, "out of fuel"),
            HostError::BadRequest(detail) => write!(f, "bad request: {detail}"),
            HostError::Failed(detail) => write!(f, "{detail}"),
        }
    }
}

impl wasmi::errors::HostError for HostError {}

impl From<HostError> for Error {
    fn from(error: HostError) -> Error {
        Error::host(error)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Changes {
    pub ui: Vec<UiChange>,
    pub rejected: Vec<String>,
}

/// The data of an instance's store.
pub struct HostState {
    pub context: PluginContext,
    pub(crate) limiter: Limiter,
    changes: Changes,
    ui_sets: u32,
    /// Fuel the imports charged in the current slice, on top of what the module burned.
    charge: u64,
    allowance: u64,
    used_before_slice: u64,
    slice: u64,
}

impl HostState {
    pub(crate) fn new(context: PluginContext, limiter: Limiter) -> HostState {
        HostState {
            context,
            limiter,
            changes: Changes::default(),
            ui_sets: 0,
            charge: 0,
            allowance: u64::MAX,
            used_before_slice: 0,
            slice: 0,
        }
    }

    pub(crate) fn begin_event(&mut self) {
        self.ui_sets = 0;
        self.changes = Changes::default();
        // A refusal of an earlier event, which the plugin may have handled, says nothing of a
        // trap in this one.
        self.limiter.denied_by_pool = false;
    }

    pub(crate) fn begin_slice(&mut self, allowance: u64, used: u64, slice: u64) {
        self.allowance = allowance;
        self.used_before_slice = used;
        self.slice = slice;
        self.charge = 0;
    }

    pub(crate) fn take_charge(&mut self) -> u64 {
        std::mem::take(&mut self.charge)
    }

    pub(crate) fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }
}

/// The capability an import needs and its type, or `None` for an import Bientôt does not offer.
pub fn import(module: &str, name: &str) -> Option<(Option<Capability>, FuncType)> {
    let capability = match (module, name) {
        ("bt:log@1", "write") | ("bt:time@1", "now") => None,
        ("bt:day@1", "current") => Some(Capability::ReadDay),
        ("bt:ui@1", "set" | "clear") => Some(Capability::UiFlyout),
        _ => return None,
    };
    Some((
        capability,
        FuncType::new([ValType::I32, ValType::I32], [ValType::I64]),
    ))
}

pub(crate) fn define(linker: &mut Linker<HostState>) -> Result<(), Error> {
    linker.func_wrap(
        "bt:log@1",
        "write",
        |c: Caller<'_, HostState>, p: i32, l: i32| serve(c, p, l, log_write),
    )?;
    linker.func_wrap(
        "bt:time@1",
        "now",
        |c: Caller<'_, HostState>, p: i32, l: i32| serve(c, p, l, time_now),
    )?;
    linker.func_wrap(
        "bt:day@1",
        "current",
        |c: Caller<'_, HostState>, p: i32, l: i32| serve(c, p, l, day_current),
    )?;
    linker.func_wrap(
        "bt:ui@1",
        "set",
        |c: Caller<'_, HostState>, p: i32, l: i32| serve(c, p, l, ui_set),
    )?;
    linker.func_wrap(
        "bt:ui@1",
        "clear",
        |c: Caller<'_, HostState>, p: i32, l: i32| serve(c, p, l, ui_clear),
    )?;
    Ok(())
}

type Handler = fn(&mut HostState, Value) -> Result<Option<Value>, HostError>;

/// Reads the request, runs `handler` and hands its answer back through `bt_alloc`.
fn serve(
    mut caller: Caller<'_, HostState>,
    ptr: i32,
    len: i32,
    handler: Handler,
) -> Result<i64, Error> {
    let (ptr, len) = (ptr as u32 as usize, len as u32 as usize);
    if len > MAX_REQUEST_BYTES {
        return Err(HostError::BadRequest("a request is larger than 64 KiB".into()).into());
    }
    charge(&mut caller, IMPORT_FUEL + FUEL_PER_BYTE * len as u64)?;
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => memory,
        _ => return Err(HostError::Failed("no memory".into()).into()),
    };
    let mut request = vec![0; len];
    memory
        .read(&caller, ptr, &mut request)
        .map_err(|_| HostError::BadRequest("the request is outside memory".into()))?;
    let request = parse_request(&request)?;
    let Some(answer) = handler(caller.data_mut(), request)? else {
        return Ok(0);
    };
    let answer = serde_json::to_vec(&answer).map_err(|e| HostError::Failed(e.to_string()))?;
    give(&mut caller, &answer)
}

fn parse_request(bytes: &[u8]) -> Result<Value, HostError> {
    if depth(bytes) > MAX_REQUEST_DEPTH {
        return Err(HostError::BadRequest(
            "a request is nested deeper than 32 levels".into(),
        ));
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) if value.is_object() => Ok(value),
        _ => Err(HostError::BadRequest(
            "a request is not a JSON object".into(),
        )),
    }
}

/// The deepest nesting of arrays and objects, outside strings.
fn depth(bytes: &[u8]) -> usize {
    let (mut depth, mut deepest, mut in_string, mut escaped) = (0usize, 0, false, false);
    for &b in bytes {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Charges fuel to the current call, failing when that would pass its allowance.
fn charge(caller: &mut Caller<'_, HostState>, fuel: u64) -> Result<(), Error> {
    let left = caller.get_fuel().unwrap_or(0);
    let state = caller.data_mut();
    let used = state.used_before_slice + state.slice.saturating_sub(left) + state.charge;
    if used.saturating_add(fuel) > state.allowance {
        return Err(HostError::OutOfFuel.into());
    }
    state.charge += fuel;
    Ok(())
}

/// Copies `answer` into a buffer the module allocates, which then belongs to the module.
fn give(caller: &mut Caller<'_, HostState>, answer: &[u8]) -> Result<i64, Error> {
    charge(caller, FUEL_PER_BYTE * answer.len() as u64)?;
    let alloc = match caller.get_export("bt_alloc") {
        Some(Extern::Func(func)) => func.typed::<i32, i32>(&caller)?,
        _ => return Err(HostError::Failed("no bt_alloc".into()).into()),
    };
    let fuel = caller.get_fuel()?;
    caller.set_fuel(fuel + NESTED_FUEL)?;
    let ptr = alloc.call(&mut *caller, answer.len() as i32);
    let after = caller.get_fuel()?;
    caller.set_fuel(fuel)?;
    charge(caller, (fuel + NESTED_FUEL).saturating_sub(after))?;
    let ptr = ptr?;
    if ptr == 0 {
        return Err(HostError::Failed("bt_alloc returned 0".into()).into());
    }
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => memory,
        _ => return Err(HostError::Failed("no memory".into()).into()),
    };
    memory
        .write(&mut *caller, ptr as u32 as usize, answer)
        .map_err(|_| HostError::BadRequest("bt_alloc returned a buffer outside memory".into()))?;
    Ok(((ptr as u32 as i64) << 32) | answer.len() as i64)
}

#[derive(Deserialize)]
struct LogRequest {
    level: LogLevel,
    text: String,
}

fn log_write(state: &mut HostState, request: Value) -> Result<Option<Value>, HostError> {
    let request: LogRequest = serde_json::from_value(request)
        .map_err(|e| HostError::BadRequest(format!("bt:log@1 write: {e}")))?;
    // The context's log is a [`LimitedLog`] wherever Bientôt runs plugins: it keeps the rate.
    let text: String = request.text.chars().take(MAX_LOG_CHARS).collect();
    state
        .context
        .log
        .write(&state.context.id, request.level, &text);
    Ok(None)
}

fn time_now(state: &mut HostState, _: Value) -> Result<Option<Value>, HostError> {
    Ok(Some(json!({ "now": (state.context.clock)().to_string() })))
}

fn day_current(state: &mut HostState, _: Value) -> Result<Option<Value>, HostError> {
    Ok(Some(match &state.context.day {
        Some(day) => serde_json::from_str(day).map_err(|e| HostError::Failed(e.to_string()))?,
        None => json!({ "error": "not-ready" }),
    }))
}

fn slot(request: &Value) -> Result<(), HostError> {
    match request.get("slot").and_then(Value::as_str) {
        Some(CARD_SLOT) => Ok(()),
        _ => Err(HostError::BadRequest(
            "bt:ui@1: the only slot is `flyout.card`".into(),
        )),
    }
}

fn ui_set(state: &mut HostState, mut request: Value) -> Result<Option<Value>, HostError> {
    slot(&request)?;
    if state.ui_sets >= MAX_UI_SETS {
        return Ok(Some(json!({ "error": "at most 2 updates per event" })));
    }
    state.ui_sets += 1;
    let tree = request
        .get_mut("tree")
        .map(Value::take)
        .ok_or_else(|| HostError::BadRequest("bt:ui@1 set: no `tree`".into()))?;
    let checked = Card::from_value(tree).and_then(|card| {
        let context = ViewContext {
            texts: state.context.texts(),
            manifest: &state.context.manifest,
        };
        card.prepare(&context).map(|_| card)
    });
    Ok(Some(match checked {
        Ok(card) => {
            state.changes.ui.push(UiChange::Set(card));
            json!({ "ok": true })
        }
        Err(error) => {
            let message = error.to_string();
            state.changes.rejected.push(message.clone());
            json!({ "error": message })
        }
    }))
}

fn ui_clear(state: &mut HostState, request: Value) -> Result<Option<Value>, HostError> {
    slot(&request)?;
    state.changes.ui.push(UiChange::Clear);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::runtime::tests::Lines;

    #[test]
    fn a_log_keeps_60_lines_a_minute_and_reports_the_rest() {
        let lines = Arc::new(Lines::default());
        let log = LimitedLog::new(lines.clone());
        let id = PluginId::parse("bientot.focus").unwrap();
        let t0 = Instant::now();
        for _ in 0..70 {
            log.write_at(&id, LogLevel::Info, "x", t0);
        }
        let taken = |lines: &Lines| std::mem::take(&mut *lines.0.lock().unwrap());
        assert_eq!(taken(&lines).len(), 60);
        // The next minute starts with the count of the last one.
        log.write_at(&id, LogLevel::Info, "x", t0 + Duration::from_secs(61));
        assert_eq!(taken(&lines), ["dropped 10", "bientot.focus Info x"]);
        for _ in 0..65 {
            log.write_at(&id, LogLevel::Info, "x", t0 + Duration::from_secs(62));
        }
        assert_eq!(taken(&lines).len(), 59);
        // A plugin that falls silent or goes still has its count written.
        drop(log);
        assert_eq!(taken(&lines), ["dropped 6"]);
    }
}
