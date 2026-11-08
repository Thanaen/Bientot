use std::sync::Mutex;

use bientot_plugin_api::Manifest;
use serde_json::json;

use super::*;
use crate::host::{LogLevel, LogSink};

#[derive(Default)]
pub(crate) struct Lines(pub Mutex<Vec<String>>);

impl LogSink for Lines {
    fn write(&self, plugin: &bientot_plugin_api::PluginId, level: LogLevel, text: &str) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {level:?} {text}", plugin.as_str()));
    }

    fn dropped(&self, _: &bientot_plugin_api::PluginId, count: u32) {
        self.0.lock().unwrap().push(format!("dropped {count}"));
    }

    fn host(&self, _: &bientot_plugin_api::PluginId, text: &str) {
        self.0.lock().unwrap().push(format!("host {text}"));
    }
}

pub(crate) fn manifest(memory: &str, memory_min: &str) -> Arc<Manifest> {
    let value = json!({
        "id": "bientot.focus",
        "name": { "en": "Focus" },
        "version": "0.1.0",
        "api": "1.0",
        "memory": memory,
        "memory_min": memory_min,
        "locales": ["en"],
        "capabilities": ["read:day", "ui:flyout"]
    });
    Arc::new(Manifest::parse(value.to_string().as_bytes(), &[]).unwrap())
}

pub(crate) fn context(manifest: Arc<Manifest>, log: Arc<Lines>) -> PluginContext {
    let texts = bientot_plugin_api::LocaleTexts::from_pairs([("title", "Focus")]);
    PluginContext {
        id: manifest.id.clone(),
        manifest,
        locales: Arc::new([("en".to_owned(), texts)].into_iter().collect()),
        language: bientot_plugin_api::Language::En,
        day: None,
        log,
        clock: Arc::new(|| "2026-11-08T12:00:00Z".parse().unwrap()),
    }
}

fn data(offset: u32, bytes: &[u8]) -> String {
    let escaped: String = bytes.iter().map(|b| format!("\\{b:02x}")).collect();
    format!("(data (i32.const {offset}) \"{escaped}\")")
}

/// Calls `import` with the JSON at `offset`, dropping the answer.
pub(crate) fn call(import: &str, offset: u32, text: &str) -> String {
    format!(
        "(drop (call ${import} (i32.const {offset}) (i32.const {})))",
        text.len()
    )
}

pub(crate) const IMPORTS: &str = r#"
  (import "bt:time@1" "now" (func $now (param i32 i32) (result i64)))
  (import "bt:log@1" "write" (func $log (param i32 i32) (result i64)))
  (import "bt:day@1" "current" (func $day (param i32 i32) (result i64)))
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (import "bt:ui@1" "clear" (func $clear (param i32 i32) (result i64)))
"#;

/// A module with a bump allocator, `datas` and `handle` as the body of `bt_handle`.
pub(crate) fn module(imports: &str, datas: &[(u32, &str)], handle: &str, extra: &str) -> Vec<u8> {
    let datas: String = datas.iter().map(|(o, t)| data(*o, t.as_bytes())).collect();
    let text = format!(
        r#"(module {imports}
  (memory (export "memory") 1 64)
  (global $heap (mut i32) (i32.const 32768))
  {datas}
  (func (export "bt_abi") (result i32) i32.const 1)
  (func (export "bt_alloc") (param i32) (result i32) (local i32)
    global.get $heap
    local.set 1
    global.get $heap
    local.get 0
    i32.add
    global.set $heap
    local.get 1)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64) {handle})
  {extra})"#
    );
    wat::parse_str(text).unwrap()
}

const LOG: &str = r#"{"level":"info","text":"hello"}"#;
pub(crate) const TREE: &str =
    r#"{"slot":"flyout.card","tree":{"type":"card","lines":[{"type":"text","key":"title"}]}}"#;
const BAD_TREE: &str =
    r#"{"slot":"flyout.card","tree":{"type":"card","lines":[{"type":"text","key":"nope"}]}}"#;

fn granted() -> BTreeSet<Capability> {
    BTreeSet::from([Capability::ReadDay, Capability::UiFlyout])
}

fn generous() -> Limits {
    Limits {
        allowance: 50_000_000,
        safety_net: Duration::from_secs(60),
    }
}

fn start(wasm: &[u8]) -> Result<PluginInstance, CallError> {
    let runtime = Runtime::default();
    let module = runtime.compile(wasm, &granted()).unwrap();
    runtime.instantiate(&module, context(manifest("8MiB", "1MiB"), Arc::default()))
}

fn deliver(instance: &mut PluginInstance) -> Result<Outcome, CallError> {
    instance.deliver(
        br#"{"event":"activate","language":"en","reason":"first"}"#,
        generous(),
        &mut || {},
    )
}

#[test]
fn a_plugin_logs_reads_the_time_and_sets_its_card() {
    let wasm = module(
        IMPORTS,
        &[(0, "{}"), (16, LOG), (64, TREE)],
        &format!(
            "{} {} {} i64.const 0",
            call("now", 0, "{}"),
            call("log", 16, LOG),
            call("set", 64, TREE)
        ),
        "",
    );
    let runtime = Runtime::default();
    let module = runtime.compile(&wasm, &granted()).unwrap();
    let lines = Arc::new(Lines::default());
    let mut instance = runtime
        .instantiate(&module, context(manifest("8MiB", "1MiB"), lines.clone()))
        .unwrap();
    let outcome = deliver(&mut instance).unwrap();
    assert_eq!(outcome.ui.len(), 1);
    assert!(matches!(&outcome.ui[0], UiChange::Set(card) if card.lines.len() == 1));
    assert!(outcome.rejected.is_empty());
    assert!(outcome.fuel > 3 * crate::host::IMPORT_FUEL);
    assert_eq!(*lines.0.lock().unwrap(), vec!["bientot.focus Info hello"]);
    assert_eq!(runtime.pool().used(), 1 << 20);
    drop(instance);
    assert_eq!(runtime.pool().used(), 0);
}

#[test]
fn a_rejected_tree_is_reported_and_the_instance_lives() {
    let wasm = module(
        IMPORTS,
        &[(64, BAD_TREE)],
        &format!("{} i64.const 0", call("set", 64, BAD_TREE)),
        "",
    );
    let mut instance = start(&wasm).unwrap();
    let outcome = deliver(&mut instance).unwrap();
    assert!(outcome.ui.is_empty());
    assert_eq!(outcome.rejected.len(), 1);
    assert!(deliver(&mut instance).is_ok());
}

#[test]
fn at_most_two_card_updates_per_event() {
    let set = call("set", 64, TREE);
    let wasm = module(
        IMPORTS,
        &[(64, TREE)],
        &format!("{set} {set} {set} i64.const 0"),
        "",
    );
    let outcome = deliver(&mut start(&wasm).unwrap()).unwrap();
    assert_eq!(outcome.ui.len(), 2);
}

#[test]
fn the_day_answers_not_ready_before_the_first_refresh() {
    // Writes the answer's length to address 0, then returns it as the response.
    let handle = format!(
        "(local i64) (local.set 2 {}) (local.get 2)",
        "(call $day (i32.const 0) (i32.const 2))"
    );
    let wasm = module(IMPORTS, &[(0, "{}")], &handle, "");
    let mut instance = start(&wasm).unwrap();
    assert!(deliver(&mut instance).is_ok());
}

#[test]
fn a_response_must_be_a_json_object() {
    let response = "[1, 2]";
    let handle = format!(
        "(i64.or (i64.shl (i64.const 128) (i64.const 32)) (i64.const {}))",
        response.len()
    );
    let wasm = module(IMPORTS, &[(128, response)], &handle, "");
    assert!(matches!(
        deliver(&mut start(&wasm).unwrap()),
        Err(CallError::BadData(_))
    ));
    let response = r#"{"later":"field"}"#;
    let handle = format!(
        "(i64.or (i64.shl (i64.const 128) (i64.const 32)) (i64.const {}))",
        response.len()
    );
    let wasm = module(IMPORTS, &[(128, response)], &handle, "");
    assert!(deliver(&mut start(&wasm).unwrap()).is_ok());
}

#[test]
fn bad_requests_stop_the_call() {
    let not_json = "{oops";
    let wasm = module(
        IMPORTS,
        &[(0, not_json)],
        &format!("{} i64.const 0", call("log", 0, not_json)),
        "",
    );
    assert!(matches!(
        deliver(&mut start(&wasm).unwrap()),
        Err(CallError::BadData(_))
    ));
    let deep = format!("{{\"a\":{}{}}}", "[".repeat(40), "]".repeat(40));
    let wasm = module(
        IMPORTS,
        &[(0, &deep)],
        &format!("{} i64.const 0", call("log", 0, &deep)),
        "",
    );
    assert!(matches!(
        deliver(&mut start(&wasm).unwrap()),
        Err(CallError::BadData(_))
    ));
    let wasm = module(
        IMPORTS,
        &[],
        "(drop (call $log (i32.const 0) (i32.const 70000))) i64.const 0",
        "",
    );
    assert!(matches!(
        deliver(&mut start(&wasm).unwrap()),
        Err(CallError::BadData(_))
    ));
    let other_slot = r#"{"slot":"tray"}"#;
    let wasm = module(
        IMPORTS,
        &[(0, other_slot)],
        &format!("{} i64.const 0", call("clear", 0, other_slot)),
        "",
    );
    assert!(matches!(
        deliver(&mut start(&wasm).unwrap()),
        Err(CallError::BadData(_))
    ));
}

#[test]
fn traps_are_failures() {
    let wasm = module("", &[], "unreachable", "");
    let error = deliver(&mut start(&wasm).unwrap()).unwrap_err();
    assert!(matches!(error, CallError::Trap(_)) && error.is_failure());
    let wasm = module(
        "",
        &[],
        "(drop (i32.load (i32.const 99999999))) i64.const 0",
        "",
    );
    assert!(matches!(
        deliver(&mut start(&wasm).unwrap()),
        Err(CallError::Trap(_))
    ));
}

const SPIN: &str = "(loop $l (br $l)) i64.const 0";

#[test]
fn a_loop_runs_in_slices_until_its_fuel_is_spent() {
    let wasm = module("", &[], SPIN, "");
    let mut instance = start(&wasm).unwrap();
    let mut pauses = 0;
    let limits = Limits {
        allowance: 3 * SLICE,
        safety_net: Duration::from_secs(60),
    };
    let error = instance
        .deliver(b"{}", limits, &mut || pauses += 1)
        .unwrap_err();
    assert_eq!(error, CallError::OutOfFuel);
    assert!(error.is_failure());
    // The alloc call takes a slice of its own; the loop then gets the rest.
    assert!(pauses >= 1);
}

#[test]
fn the_safety_net_stops_a_slow_call_without_a_failure() {
    let wasm = module("", &[], SPIN, "");
    let limits = Limits {
        allowance: u64::MAX / 2,
        safety_net: Duration::from_micros(1),
    };
    let error = start(&wasm)
        .unwrap()
        .deliver(b"{}", limits, &mut || {})
        .unwrap_err();
    assert_eq!(error, CallError::TimedOut);
    assert!(!error.is_failure());
}

#[test]
fn imports_charge_fuel() {
    // 200 log calls with no fuel for them.
    let body = format!(
        "(local i32) (loop $l {} (local.set 2 (i32.add (local.get 2) (i32.const 1))) (br_if $l (i32.lt_u (local.get 2) (i32.const 200)))) i64.const 0",
        call("log", 0, LOG)
    );
    let wasm = module(IMPORTS, &[(0, LOG)], &body, "");
    let limits = Limits {
        allowance: 1_000_000,
        safety_net: Duration::from_secs(60),
    };
    assert_eq!(
        start(&wasm)
            .unwrap()
            .deliver(b"{}", limits, &mut || {})
            .unwrap_err(),
        CallError::OutOfFuel
    );
}

#[test]
fn memory_is_held_to_the_declaration_and_the_pool() {
    // Grows by `pages` and returns the result of memory.grow in the response length… as a trap
    // when it fails.
    let grow = |pages: u32| {
        module(
            "",
            &[],
            &format!(
                "(if (i32.eq (memory.grow (i32.const {pages})) (i32.const -1)) (then unreachable)) i64.const 0"
            ),
            "",
        )
    };
    // 8 MiB declared: 63 more pages of 64 KiB pass, 200 do not.
    assert!(deliver(&mut start(&grow(63)).unwrap()).is_ok());
    assert!(matches!(
        deliver(&mut start(&grow(200)).unwrap()),
        Err(CallError::Trap(_))
    ));

    let pool = Pool::new(3 << 20);
    let runtime = Runtime::new(pool.clone());
    let module = runtime.compile(&grow(40), &granted()).unwrap();
    let make = || runtime.instantiate(&module, context(manifest("8MiB", "1MiB"), Arc::default()));
    let (mut a, b, c) = (make().unwrap(), make().unwrap(), make().unwrap());
    assert_eq!(make().err(), Some(CallError::PoolFull));
    // 41 pages are 2.6 MiB, more than the 1 MiB `a` holds while `b` and `c` hold the rest.
    let error = deliver(&mut a).unwrap_err();
    assert_eq!(error, CallError::PoolFull);
    assert!(!error.is_failure());
    drop((a, b, c));
    assert_eq!(pool.used(), 0);
    let mut alone = make().unwrap();
    assert!(deliver(&mut alone).is_ok());
    assert_eq!(pool.used(), 41 << 16);
}

#[test]
fn a_handled_pool_refusal_does_not_hide_a_later_trap() {
    // `{}` asks for 40 pages and ignores the refusal; anything longer traps.
    let wasm = module(
        "",
        &[],
        "(if (i32.eq (local.get 1) (i32.const 2)) \
            (then (drop (memory.grow (i32.const 40)))) (else unreachable)) i64.const 0",
        "",
    );
    let pool = Pool::new(3 << 20);
    let runtime = Runtime::new(pool);
    let module = runtime.compile(&wasm, &granted()).unwrap();
    let make = || runtime.instantiate(&module, context(manifest("8MiB", "1MiB"), Arc::default()));
    let (mut a, _b, _c) = (make().unwrap(), make().unwrap(), make().unwrap());
    let limits = Limits::for_event(CallKind::Background, 2);
    assert!(a.deliver(b"{}", limits, &mut || {}).is_ok());
    let error = a.deliver(b"{ }", limits, &mut || {}).unwrap_err();
    assert!(matches!(error, CallError::Trap(_)), "{error:?}");
    assert!(error.is_failure());
}

#[test]
fn modules_are_checked() {
    let runtime = Runtime::default();
    let check =
        |wasm: Vec<u8>, granted: BTreeSet<Capability>| runtime.compile(&wasm, &granted).err();
    assert!(matches!(
        check(b"nope".to_vec(), granted()),
        Some(ModuleError::Invalid(_))
    ));
    let no_handle = wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "bt_abi") (result i32) i32.const 1))"#).unwrap();
    assert_eq!(
        check(no_handle, granted()),
        Some(ModuleError::MissingExport("bt_alloc"))
    );
    let wrong = module("", &[], "i64.const 0", "").to_vec();
    assert!(check(wrong, granted()).is_none());
    let unknown = module(
        r#"(import "env" "fetch" (func (param i32 i32) (result i64)))"#,
        &[],
        "i64.const 0",
        "",
    );
    assert_eq!(
        check(unknown, granted()),
        Some(ModuleError::UnknownImport("env.fetch".into()))
    );
    let wrong_type = module(
        r#"(import "bt:log@1" "write" (func (param i32) (result i64)))"#,
        &[],
        "i64.const 0",
        "",
    );
    assert!(matches!(
        check(wrong_type, granted()),
        Some(ModuleError::UnknownImport(_))
    ));
    let only_day = BTreeSet::from([Capability::ReadDay]);
    assert_eq!(
        check(module(IMPORTS, &[], "i64.const 0", ""), only_day),
        Some(ModuleError::NotGranted(Capability::UiFlyout))
    );
    let later = module(
        r#"(import "bt:net@1" "fetch" (func (param i32 i32) (result i64)))"#,
        &[],
        "i64.const 0",
        "",
    );
    assert!(matches!(
        check(later, granted()),
        Some(ModuleError::UnknownImport(_))
    ));
}

#[test]
fn the_abi_and_the_start_function_are_checked() {
    let abi2 = wat::parse_str(
        r#"(module (memory (export "memory") 1)
        (func (export "bt_abi") (result i32) i32.const 2)
        (func (export "bt_alloc") (param i32) (result i32) i32.const 8)
        (func (export "bt_free") (param i32 i32))
        (func (export "bt_handle") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    assert_eq!(start(&abi2).err(), Some(CallError::BadAbi(2)));
    let spinning_start = module(
        "",
        &[],
        "i64.const 0",
        "(func $s (loop $l (br $l))) (start $s)",
    );
    assert_eq!(start(&spinning_start).err(), Some(CallError::OutOfFuel));
}
