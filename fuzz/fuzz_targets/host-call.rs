//! The requests a plugin sends to the host imports: the first byte of the input picks the import,
//! the rest is the request, sent from inside a real WebAssembly instance. No request panics the
//! host, and the host refuses a bad one as bad data, never as a failure of its own.

#![no_main]

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bientot_plugin_api::{Language, LocaleTexts, Manifest, PluginId};
use bientot_plugins::host::{LogLevel, LogSink, PluginContext};
use bientot_plugins::runtime::{CallError, Limits, Module, PluginInstance, Runtime};
use libfuzzer_sys::fuzz_target;

/// `bt_handle` calls the import the first byte picks with the bytes after it, then frees every
/// buffer, so that the instance lives across inputs.
const MODULE: &str = r#"(module
  (import "bt:log@1" "write" (func $log (param i32 i32) (result i64)))
  (import "bt:time@1" "now" (func $now (param i32 i32) (result i64)))
  (import "bt:day@1" "current" (func $day (param i32 i32) (result i64)))
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (import "bt:ui@1" "clear" (func $clear (param i32 i32) (result i64)))
  (memory (export "memory") 2 64)
  (global $heap (mut i32) (i32.const 1024))
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
  (func (export "bt_handle") (param $ptr i32) (param $len i32) (result i64) (local $at i32)
    (if (i32.eqz (local.get $len)) (then (return (i64.const 0))))
    (local.set $at (i32.add (local.get $ptr) (i32.const 1)))
    (block $done
      (block $clear (block $set (block $day (block $now (block $log
        (br_table $log $now $day $set $clear
          (i32.rem_u (i32.load8_u (local.get $ptr)) (i32.const 5))))
        (drop (call $log (local.get $at) (i32.sub (local.get $len) (i32.const 1))))
        (br $done))
        (drop (call $now (local.get $at) (i32.sub (local.get $len) (i32.const 1))))
        (br $done))
        (drop (call $day (local.get $at) (i32.sub (local.get $len) (i32.const 1))))
        (br $done))
        (drop (call $set (local.get $at) (i32.sub (local.get $len) (i32.const 1))))
        (br $done))
      (drop (call $clear (local.get $at) (i32.sub (local.get $len) (i32.const 1)))))
    (global.set $heap (i32.const 1024))
    i64.const 0))"#;

struct Quiet;

impl LogSink for Quiet {
    fn write(&self, _: &PluginId, _: LogLevel, _: &str) {}
    fn dropped(&self, _: &PluginId, _: u32) {}
    fn host(&self, _: &PluginId, _: &str) {}
}

struct Compiled {
    runtime: Runtime,
    module: Module,
    manifest: Arc<Manifest>,
}

/// Compiled once; each refused request costs only a new instance.
fn compiled() -> &'static Compiled {
    static COMPILED: OnceLock<Compiled> = OnceLock::new();
    COMPILED.get_or_init(|| {
        let manifest = Arc::new(
            Manifest::parse(
                br#"{"id":"bientot.focus","name":{"en":"Focus"},"version":"0.1.0","api":"1.0",
                    "memory":"4MiB","memory_min":"128KiB","locales":["en"],
                    "capabilities":["read:day","ui:flyout"]}"#,
                &[],
            )
            .expect("the fuzz manifest parses"),
        );
        let runtime = Runtime::default();
        let wasm = wat::parse_str(MODULE).expect("the fuzz module assembles");
        let module = runtime
            .compile(&wasm, &manifest.capabilities)
            .expect("the fuzz module compiles");
        Compiled {
            runtime,
            module,
            manifest,
        }
    })
}

fn instance() -> PluginInstance {
    let Compiled {
        runtime,
        module,
        manifest,
    } = compiled();
    let context = PluginContext {
        id: manifest.id.clone(),
        locales: Arc::new(
            [(
                "en".to_owned(),
                LocaleTexts::from_pairs([("title", "Focus")]),
            )]
            .into_iter()
            .collect(),
        ),
        manifest: manifest.clone(),
        language: Language::En,
        day: Some(r#"{"date":"2026-11-08","worked":3600}"#.into()),
        log: Arc::new(Quiet),
        clock: Arc::new(|| "2026-11-08T12:00:00Z".parse().expect("a time")),
    };
    runtime
        .instantiate(module, context)
        .expect("the fuzz module starts")
}

fuzz_target!(|bytes: &[u8]| {
    static INSTANCE: Mutex<Option<PluginInstance>> = Mutex::new(None);
    let mut slot = INSTANCE.lock().unwrap_or_else(|e| e.into_inner());
    let plugin = slot.get_or_insert_with(instance);
    let limits = Limits {
        allowance: 100_000_000,
        safety_net: Duration::from_secs(60),
    };
    match plugin.deliver(bytes, limits, &mut || {}) {
        Ok(_) => {}
        Err(CallError::BadData(_)) => *slot = None,
        Err(error) => panic!("the host failed on a request: {error}"),
    }
});
