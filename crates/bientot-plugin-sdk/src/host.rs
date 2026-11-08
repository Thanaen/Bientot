//! The host calls of api 1.0 (contract `host-abi.md`, « Imports »). Each one sends the JSON
//! request the contract names and reads its answer. In a WebAssembly build the request goes to
//! the import; in a native build it goes to the [`crate::testing::TestHost`] running the plugin.
//!
//! A module imports only the calls it uses, and Bientôt refuses a module whose imports need a
//! capability its manifest does not declare: [`day::current`] needs `read:day`, [`ui::show`] and
//! [`ui::clear`] need `ui:flyout`.

use serde_json::{Value, json};

/// The five imports, named for the native test host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Import {
    LogWrite,
    TimeNow,
    DayCurrent,
    UiSet,
    UiClear,
}

impl Import {
    /// `module.name`, as in the module's import section.
    pub fn name(self) -> &'static str {
        match self {
            Import::LogWrite => "bt:log@1.write",
            Import::TimeNow => "bt:time@1.now",
            Import::DayCurrent => "bt:day@1.current",
            Import::UiSet => "bt:ui@1.set",
            Import::UiClear => "bt:ui@1.clear",
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "bt:log@1")]
    unsafe extern "C" {
        #[link_name = "write"]
        pub fn log_write(ptr: i32, len: i32) -> i64;
    }

    #[link(wasm_import_module = "bt:time@1")]
    unsafe extern "C" {
        #[link_name = "now"]
        pub fn time_now(ptr: i32, len: i32) -> i64;
    }

    #[link(wasm_import_module = "bt:day@1")]
    unsafe extern "C" {
        #[link_name = "current"]
        pub fn day_current(ptr: i32, len: i32) -> i64;
    }

    #[link(wasm_import_module = "bt:ui@1")]
    unsafe extern "C" {
        #[link_name = "set"]
        pub fn ui_set(ptr: i32, len: i32) -> i64;
        #[link_name = "clear"]
        pub fn ui_clear(ptr: i32, len: i32) -> i64;
    }

    /// Calls one import and takes the answer buffer, which the host allocated with `bt_alloc`.
    pub fn call(import: unsafe extern "C" fn(i32, i32) -> i64, request: &[u8]) -> Vec<u8> {
        // SAFETY: the request outlives the call; the host only reads it.
        let packed = unsafe { import(request.as_ptr() as usize as i32, request.len() as i32) };
        let (ptr, len) = crate::abi::split(packed);
        // SAFETY: a non-empty answer is a buffer the host made with `bt_alloc` and filled.
        unsafe { crate::abi::take(ptr, len) }
    }
}

/// Sends `request` to one import. Each WebAssembly build names its own import, so that a module
/// that never shows a card does not import `bt:ui@1`.
macro_rules! call {
    ($wasm:ident, $native:ident, $request:expr) => {{
        let request = serde_json::to_vec(&$request).expect("a JSON value serializes");
        #[cfg(target_arch = "wasm32")]
        let answer = imports::call(imports::$wasm, &request);
        #[cfg(not(target_arch = "wasm32"))]
        let answer = crate::testing::dispatch(Import::$native, &request);
        answer
    }};
}

fn read(answer: &[u8]) -> Value {
    if answer.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(answer).unwrap_or(Value::Null)
}

/// `bt:log@1`: lines in the plugin's own log file, `logs\plugins\<id>.jsonl`. At most 60 lines a
/// minute of at most 2 048 characters; the rest is dropped and counted. The log stays on the
/// user's machine and is not part of a diagnostics export.
pub mod log {
    use super::*;

    fn write(level: &str, text: &str) {
        call!(log_write, LogWrite, json!({ "level": level, "text": text }));
    }

    pub fn info(text: &str) {
        write("info", text);
    }

    pub fn warn(text: &str) {
        write("warn", text);
    }

    pub fn error(text: &str) {
        write("error", text);
    }
}

/// `bt:time@1`: the host's clock.
pub mod time {
    use super::*;
    use jiff::Timestamp;

    /// Now, as the host sees it. In tests, the time the test host was set to.
    pub fn now() -> Timestamp {
        let answer = read(&call!(time_now, TimeNow, json!({})));
        answer["now"]
            .as_str()
            .and_then(|text| text.parse().ok())
            .unwrap_or(Timestamp::UNIX_EPOCH)
    }
}

/// `bt:day@1`: the current day. Needs `read:day`.
pub mod day {
    use super::*;
    use bientot_plugin_api::DaySnapshot;

    /// The day as of the last calculation, or `None` before the first one after a start.
    pub fn current() -> Option<DaySnapshot> {
        let answer = call!(day_current, DayCurrent, json!({}));
        serde_json::from_slice(&answer).ok()
    }
}

/// `bt:ui@1`: the plugin's card in the flyout. Needs `ui:flyout`.
pub mod ui {
    use super::*;
    use bientot_plugin_api::Card;

    const SLOT: &str = "flyout.card";

    /// Shows `card` in the flyout, in place of the previous one. Bientôt checks the card
    /// (`contracts/view-tree.md`) and keeps the previous one when it refuses it, with the reason
    /// in `Err`; a refusal counts as a failure of the plugin. At most two calls per event.
    pub fn show(card: impl Into<Card>) -> Result<(), String> {
        let tree = serde_json::to_value(card.into()).expect("a card serializes");
        let answer = read(&call!(ui_set, UiSet, json!({ "slot": SLOT, "tree": tree })));
        match answer["error"].as_str() {
            Some(error) => Err(error.to_owned()),
            None => Ok(()),
        }
    }

    /// Takes the card away.
    pub fn clear() {
        call!(ui_clear, UiClear, json!({ "slot": SLOT }));
    }
}
