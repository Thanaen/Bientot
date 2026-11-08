# Contract: host ABI 1 (api 1.0, this step)

Plugins are core WebAssembly modules for `wasm32-unknown-unknown`, without
the component model (which `wasmi` does not run). The contract is written in
WIT (`crates/bientot-plugin-api/wit/bientot.wit`) so that types for Rust and
TypeScript are generated from one source and a later move to components
stays possible. Payloads are JSON, UTF-8.

## Exports

| Export | Signature | Meaning |
| --- | --- | --- |
| `memory` | memory | the module's only memory |
| `bt_abi` | `() -> i32` | envelope version, `1` |
| `bt_alloc` | `(len: i32) -> i32` | allocate `len` bytes, return a pointer, `0` on failure |
| `bt_free` | `(ptr: i32, len: i32)` | free a buffer the host received |
| `bt_handle` | `(ptr: i32, len: i32) -> i64` | handle one event; return `(ptr << 32) \| len` of the JSON response, or `0` for no response |

No other export is called. A start function is allowed and runs under the
same fuel and time limits as an event.

## Buffer ownership

- The host allocates the event buffer with `bt_alloc`, writes the event,
  calls `bt_handle`. The buffer then belongs to the plugin.
- The response buffer belongs to the host once `bt_handle` returns; the host
  reads it and calls `bt_free`.
- When an import returns data, the host allocates it with `bt_alloc` and
  returns `(ptr << 32) | len`; the buffer belongs to the plugin.
- Every import takes `(ptr: i32, len: i32)` pointing at a JSON request and
  returns `i64` as above, or `0` for an empty answer.

## Imports of this step

Module names carry their version. Any other import refuses the module.

| Module | Function | Request → answer | Capability |
| --- | --- | --- | --- |
| `bt:log@1` | `write` | `{ "level": "info"\|"warn"\|"error", "text": "…" }` → empty | — |
| `bt:time@1` | `now` | `{}` → `{ "now": "<RFC 3339 UTC>" }` | — |
| `bt:day@1` | `current` | `{}` → DaySnapshot or `{ "error": "not-ready" }` | `read:day` |
| `bt:ui@1` | `set` | `{ "slot": "flyout.card", "tree": <view tree> }` → `{ "ok": true }` or `{ "error": "…" }` | `ui:flyout` |
| `bt:ui@1` | `clear` | `{ "slot": "flyout.card" }` → empty | `ui:flyout` |

Limits: a request is at most 64 KiB and 32 levels deep; `bt:log@1` accepts 60
lines per minute and per plugin, whatever its instances, of at most 2 048
characters, the rest is dropped and counted; `bt:ui@1 set` at most 2 calls per event. Each import charges fuel:
10 000 plus 250 per byte copied in either direction.

## Events

`bt_handle` receives `{ "event": "<kind>", ... }`:

| Event | Fields | When |
| --- | --- | --- |
| `installed` | `version` | once, before the first `activate` after install |
| `upgraded` | `from`, `version` | once, after an update |
| `activate` | `language`, `reason` (`first`, `evicted`, `recreated`, `restart`) | at every instantiation |
| `day.refreshed` | `day` (DaySnapshot) | structural change, at most once per 30 s |
| `command.invoked` | `command`, `arg` (string or null), `source` (`palette`, `card`) | user action |
| `language.changed` | `language` | app language changed |

An unknown event must be ignored by the plugin; the SDK does so.

## Response

`bt_handle` may return `{}` or nothing. The host reads no other field in this
step; later steps add fields, for example `continue`.

## Errors and traps

| Cause | Effect | Counts as failure |
| --- | --- | --- |
| Trap, including out-of-bounds and `unreachable` | instance discarded | yes |
| Fuel allowance exhausted | instance discarded | yes |
| Invalid JSON response or import request | instance discarded | yes |
| View tree rejected | card unchanged | yes |
| Wall-clock safety net | call stopped, instance discarded | no |
| Memory pool full after evictions | trap | no |
| `bt_abi` not `1`, missing export, unknown import | module refused at install or load | — |

After any of these, the plugin's background events wait 30 s; its user
commands do not.

## WIT excerpt

```wit
package bientot:plugin@1.0.0;

interface log {
  enum level { info, warn, error }
  write: func(level: level, text: string);
}

interface time {
  now: func() -> string; // RFC 3339, UTC
}

interface day {
  use types.{day-snapshot};
  current: func() -> result<day-snapshot, string>;
}

interface ui {
  use types.{view-node};
  set: func(slot: string, tree: view-node) -> result<_, string>;
  clear: func(slot: string);
}

world plugin {
  import log;
  import time;
  import day;
  import ui;
  export handle: func(event: string) -> option<string>;
}
```

The WIT is the source of the generated types; the core-module mapping above
is how ABI 1 carries them.
