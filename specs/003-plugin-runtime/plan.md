# Implementation Plan: Plugin runtime (first step)

**Branch**: `003-plugin-runtime` | **Date**: 2026-11-08 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/003-plugin-runtime/spec.md`

## Summary

Add a sandboxed WebAssembly plugin runtime to Bientôt and prove it with one
read-only flyout card and one palette command. Plugins are signed
`.bientot-plugin` packages read by a purpose-built reader, run by `wasmi` with
portable dispatch on a dedicated thread under fuel, memory and time budgets,
and reach the app only through four host interfaces. The host draws every
surface. A signed revocation list, a safe mode and a corpus of hostile
plugins in CI protect users. With no plugin installed the app pays nothing.

## Technical Context

**Language/Version**: Rust 1.97, edition 2024 (`rust-toolchain.toml`)

**Primary Dependencies**: existing `gpui-pre` 0.3.9, `serde_json`, `flate2`,
`ureq` (behind `usage-ping`); new `wasmi` 2.0 (pinned, `std`, `validate`,
`portable-dispatch`), `ed25519-dalek` (no default features), `sha2` (already
in `bientot-core`); the manifest is JSON, read with `serde_json` (R10)

**Storage**: files under `<data folder>\plugins\`: one folder per plugin,
`state.json` written through a temporary file and a rename like
`settings.json`, `revocations.json` (last list seen); logs under
`logs\plugins\`. No ledger change in this step.

**Testing**: `cargo test --workspace` on Linux for the API, package reader,
manifest, host, budgets and view models; `bientot-screenshots` for the card
and the Settings page; `cargo fuzz` targets; the hostile corpus and the
performance script on the `Rust on Windows` job.

**Target Platform**: Windows 11 x64 (Store MSIX, Velopack `Setup.exe`,
portable ZIP); the non-Win32 crates build and test on Linux.

**Project Type**: desktop tray application, Cargo workspace.

**Performance Goals**: card refreshed within 30 s plus one call of a
structural change; a user command served within one 2 M fuel slice (≤ 9 ms
measured) of a running background call.

**Constraints**: executable ≤ +1.0 MB and < 20 MB for `release` and
`direct`; no startup, memory or CPU change without plugins; ≤ +2 MiB idle
working set with the sample plugin; no process abort from any plugin.

**Scale/Scope**: tens of installed plugins, one visible card, four host
interfaces, nine view primitives.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Check | Status |
| --- | --- | --- |
| I. Local-first privacy | No plugin capability sends data in this step. The host's only new request is the revocation list, which sends no identifier and is ratified by D-024; `docs/privacy.md` lists it (FR-062). | Pass |
| II. Explainable accounting | Plugins only read the published day; nothing they do changes counted time. | Pass |
| III. Tray-first calm UX | One card, 4 lines and 2 buttons, after the counted time; no plugin notification in this step; card updates are not announced. | Pass |
| IV. Activity source boundary | Untouched. | Pass |
| V. Simple deterministic core | `bientot-domain` does not change. Plugins read time from the host, so plugin tests use the injected clock. Every new dependency has a stated reason (R1, R6, R7). | Pass |
| VI. Contained extensions | Sandbox, budgets, capabilities, host-drawn surfaces, no abort (hostile corpus), zero cost without plugins. | Pass |

Re-check after design: pass. The manifest is JSON (T003, `research.md` R10).

## Project Structure

### Documentation (this feature)

```text
specs/003-plugin-runtime/
├── spec.md
├── plan.md              # This file
├── research.md          # Measurements and rejected alternatives
├── data-model.md        # Entities and their states
├── quickstart.md        # Build, pack, sign and install the sample plugin
├── contracts/
│   ├── package.md       # .bientot-plugin layout, reader rules, signature
│   ├── manifest.md      # plugin.json, this step's subset
│   ├── host-abi.md      # exports, imports, events, buffers, errors, WIT
│   ├── view-tree.md     # card primitives and validation
│   └── revocations.md   # revocation list format and fetch
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
crates/
├── bientot-plugin-api/          # NEW: serde-only types shared by host and SDK
│   ├── src/lib.rs               #   versions, ids, capabilities
│   ├── src/manifest.rs          #   Manifest and its validation
│   ├── src/snapshot.rs          #   DaySnapshot v1 (hand-written DTO)
│   ├── src/event.rs             #   events of this step
│   ├── src/view.rs              #   view tree and its validation
│   ├── src/revocation.rs        #   revocation list
│   └── wit/bientot.wit          #   the contract, source of truth
├── bientot-plugins/             # NEW: the host (any OS)
│   ├── src/package.rs           #   reader, hashes, signatures, install/remove
│   ├── src/state.rs             #   plugins/state.json
│   ├── src/runtime.rs           #   wasmi engine, linker, stores, slices, fuel
│   ├── src/budget.rs            #   token buckets, failure record
│   ├── src/thread.rs            #   plugins thread, bounded queue, priorities
│   ├── src/interfaces/          #   bt:log, bt:time, bt:day, bt:ui
│   ├── src/snapshot.rs          #   WorkdayRefreshState → DaySnapshot
│   ├── src/revocation.rs        #   check, apply, persist
│   └── tests/hostile/           #   hostile corpus sources and runner
├── bientot-plugin-sdk/          # NEW: guest side, wasm32 and native test host
│   └── examples/focus/          #   the sample plugin
├── bientot-core/                # unchanged API; diagnostics export test
├── bientot-store/               # transfer: carry plugin ids and versions only
├── bientot-app/
│   ├── src/plugins.rs           #   PluginContributions global, GPUI bridge
│   ├── src/plugins/card.rs      #   prepared card → GPUI elements
│   ├── src/flyout/body.rs       #   Part::PluginCard after Part::Worked
│   ├── src/palette.rs           #   PaletteRow::Builtin | Plugin
│   ├── src/settings.rs          #   Page::Plugins
│   ├── src/settings_view/plugins.rs
│   ├── src/launch.rs            #   --no-plugins, safe mode
│   └── src/strings/table.rs     #   FR/EN texts
├── bientot-windows/             # real path of the plugins folder
fuzz/                            # targets manifest, package, host-call, view-tree
scripts/                         # pack-plugin, sign-plugin, measure gates
services/bientot/                # serve /plugins/revocations.json
```

**Structure Decision**: two new library crates and one SDK crate. The host
lives outside `bientot-app` so that it builds and tests on Linux like the
store and the core; `bientot-app` only bridges it to GPUI. `bientot-domain`
is not touched.

## Design

### Lifecycle

1. Start: if `--no-plugins` or the safe-mode marker is set, skip plugins.
   Otherwise nothing happens until `startup.tray_ready`.
2. After the tray is ready, read `plugins/state.json`. If no plugin is on,
   stop there.
3. Otherwise read each enabled plugin's manifest, check its file hashes
   (FR-005), and publish its declared commands to the palette. Create the
   `plugins` thread and the engine.
4. At the first refresh, build the snapshot once and queue `day.refreshed`
   to listening plugins, staggering first activations.
5. A plugin's first event compiles its module (lazy translation), checks its
   imports (FR-020), instantiates it with the declared minimum memory, sends
   `installed` or `upgraded` if needed, then `activate`, then the event.
6. Responses that call `bt:ui@1 set` replace the plugin's card in the
   contribution set; the GPUI side swaps the `Arc` and notifies the flyout.
7. Idle 5 min: evict. Trap: discard, record. Three traps in 10 min: off.

### Safe mode

`launch.rs` writes a `plugins/starting` marker before reading plugins and
deletes it after the first successful refresh with plugins. Two runs that
leave the marker in a row set safe mode for the next run, which starts
without plugins and shows a flyout notice with **Turn plugins back on**.

### Threads and channels

The GPUI side sends `HostRequest`s through a bounded `futures` channel
(capacity 64). The plugin thread keeps one queue per priority (user,
structural, maintenance) and coalesces `day.refreshed` per plugin. It sends
`PluginUpdate`s back through an unbounded channel drained under `cx.spawn`.
The thread never touches the ledger in this step.

### Budgets

`budget.rs` owns one token bucket per plugin and one global, refilled from a
monotonic clock. Fuel spent is converted to time with the slice's measured
duration. A call stops on fuel exhaustion (trap, failure), on the safety net
(stop, no failure) or on host refusal (failure).

### Card rendering

`bientot-plugin-api::view` validates the tree (contract `view-tree.md`).
`bientot-app/src/plugins/card.rs` converts it once into a prepared card made
of the app's widgets (`widgets.rs` texts, badges, buttons) and the theme
tokens; `Duration` and `Countdown` read the clock at render, like the
existing durations. The flyout's `Part` enum gains `PluginCard` so focus
returns correctly when the card goes away.

### Palette

`palette.rs` keeps `Command` for built-ins and adds a `PaletteRow` enum
(`Builtin(Command)` or `Plugin { plugin, command }`) for listing, filtering
and running. Plugin rows are available in the flyout and the week.

### Revocation

`bientot-plugins::revocation` verifies the list with the embedded public key,
rejects older or expired lists, applies it to `state.json`, and keeps the
last good list. `bientot-app` fetches it with the update check (behind the
same setting) and before an install or a turn-on, through `ureq` on a
background thread. Builds without `usage-ping` (debug) skip the fetch and
use the last list.

### Privacy and documentation

`docs/privacy.md` lists the request, `plugins\`, `logs\plugins\`, and the
transfer file behaviour; `docs/architecture.md` gains a « Plugins » section;
`SECURITY.md` brings the plugin host and SDK into scope.

## Complexity Tracking

| Item | Why Needed | Simpler Alternative Rejected Because |
| --- | --- | --- |
| Two new crates plus an SDK | The host must test on Linux; the API is shared by host and guests | Putting the host in `bientot-app` would tie its tests to GPUI and Windows |
| A purpose-built zip reader | Safe reading without generic extraction | `zip` 8.6 carries ignored advisories and is absent from two builds |
