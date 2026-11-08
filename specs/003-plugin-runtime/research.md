# Research: Plugin runtime

Measured on 8 November 2026 on Linux x86-64 (Xeon 2.1 GHz) with toolchain
1.97.0 and Bientôt's `[profile.release]` (`opt-level = "s"`, fat LTO, one
codegen unit, strip), plus a cross-build for `x86_64-pc-windows-msvc`.
The Windows timings (task T060) were measured the same day on an AMD Ryzen 7
9800X3D under Windows 11 build 26300, mains power, with the same bench built
for `x86_64-pc-windows-msvc`, two runs each, medians.

## R1. Engine

**Decision**: `wasmi` 2.0, pinned, `default-features = false`, features
`std`, `validate`, `portable-dispatch`.

| Measure | `wasmi` portable | `wasmi` auto | `wasmtime` (Cranelift, no WASI) |
| --- | --- | --- | --- |
| Executable growth, MSVC | +0.87 MB | +0.87 MB | not built |
| Executable growth, Linux | +0.96 MB | +0.96 MB | +6.46 MB |
| One call, 3.6 KB JSON day in and a card out, Linux | ~0.42 ms | 0.21 ms | 0.02 ms |
| Same, Windows | 0.27 ms | 0.11 ms | |
| First activation (engine, lazy compile, instantiate, first call), Linux | ~3.5 ms | ~3.1 ms | ~100 ms |
| Same, Windows | 1.4 ms | 1.1 ms | |
| Re-instantiation and one call, module kept, Windows | 0.30 ms | 0.14 ms | |
| Committed memory of an idle instance, 64 KiB stack, Windows (private bytes) | 280 KiB | 280 KiB | |
| `call_indirect` loop of 100 000 iterations, Linux and Windows | clean result | **process abort** | |

**Rationale**:

- `wasmtime` would take `worktime.exe` (16.5 MB for `direct` in v1.0.118)
  to about 23 MB, over the 20 MB gate, and compiles each module in ~100 ms.
- With `auto-dispatch`, `wasmi` picks a tail-call dispatcher on x86-64. A
  plugin looping through `call_indirect` overflowed the native stack and
  aborted the process (`fatal runtime error: stack overflow`) even on an
  8 MiB thread stack with fuel left. That is an abort, not a trap.
  `portable-dispatch` has the same size and turns the same loop into a clean
  result or trap, at about 2.3 times the call cost. On Windows the same loop
  ends the child process with `STATUS_STACK_OVERFLOW` (`0xC00000FD`) under
  `auto-dispatch` and returns in 3.6 ms under `portable-dispatch`.
- `wat` (the text format) is left out: it is not needed at run time.

**Alternatives rejected**: `wasmtime`; `wasmi` with `auto-dispatch`; Lua,
Rhai or native QuickJS (one language each, sandbox and limits to rebuild by
hand); native DLLs (no sandbox, unstable Rust ABI, Store risk);
`windows.appExtension` (Store only, every author ships a signed MSIX); MSIX
optional and modification packages (not allowed for another publisher on the
Store); external processes as the main model (no simple sandbox on Windows).

## R2. Fuel and time

50 M fuel takes 28 ms (JSON) to 122 ms (`memset`) with portable dispatch;
200 M takes 182 ms to 529 ms. On Windows, 50 M takes 34 ms (arithmetic loop)
to 103 ms (`memset`), and 200 M 135 ms to 412 ms. Fuel does not measure host
functions.

**Decision**: fuel is the limit and the clock is only a safety net (FR-013).
Calls run in 2 M slices with `call_resumable`; the longest slice measured
was 8.5 ms on Linux and 4.1 ms on Windows, so a user action waits at most
that long. Host functions charge
fuel by the bytes they copy. A wall-clock stop never counts as a failure, so
a plugin that passes on a fast PC is not turned off on a slow one.

## R3. Encoding

JSON is ~99 % of a call's cost (8.9 K fuel for an empty call, 878 K with a
3.6 KB JSON day). The same day in postcard is 998 bytes, 246 K fuel, 54 µs.

**Decision**: JSON for ABI 1: `serde_json` is already in the executable,
every language reads it, it can be read by eye. The 4× ratio is the trigger
for a binary ABI 2.

## R4. Memory

Rust links `wasm32` modules with a 1 MiB stack, which makes 17 pages at
start and ~0.6 ms of instantiation, mostly zeroing it. With
`-zstack-size=65536` a module starts at 2 pages and instantiates in 0.1 ms.
WebAssembly memory never shrinks: after a plugin frees a 12 MiB buffer, it
keeps 210 pages until the instance is dropped.

**Decision**: the SDK links with a 64 KiB stack; the host re-creates an
instance whose memory doubled (FR-017); only a declared minimum is reserved
at activation, so dozens of small plugins fit in the 32 MiB pool (FR-015).
Idle working set is 66.3 MiB today (release checklist); the worst case with
plugins stays under 100 MiB, against a 150 MiB budget.

## R5. Threading

`Engine` and `Module` are `Send + Sync`; `Store<T>` is `Send` when `T` is.
`file_dialogs.rs` already pairs a worker thread with a `futures` channel
awaited under `cx.spawn`.

**Decision**: one `plugins` thread owns the engine, modules and stores, and
sends results to the GPUI thread the same way. The contribution set is a
GPUI `Global` holding an `Arc`, replaced as a whole. Building the snapshot
costs ~10 µs natively and happens only when a plugin listens.

## R6. Package reader

The only zip crate in the tree is `zip` 8.6, pulled in by Velopack, absent
from the Store and ZIP builds, with four advisories ignored in
`osv-scanner.toml` only because `ZipArchive::extract` is never called.

**Decision**: a reader written for Bientôt: read the central directory, check
it against the local headers, inflate *stored* or *deflate* entries with
`flate2` (already a dependency of `bientot-core`), keep only allowed names.
The signature covers a canonical list of hashes, so a zip whose local headers
and central directory disagree cannot pass.

## R7. Signatures

**Decision**: ed25519 (`ed25519-dalek` 3 with `default-features = false`),
the public key in the package, bound to the id on first install. The
revocation list and future registry use the same primitive. Measured in
T061: +44 KiB on an MSVC release executable that already has `sha2` 0.11,
which `ed25519-dalek` 3 shares.

## R8. Microsoft Store

Store Policies v7.20 (effective 22 October 2026), checked by the Store
reviewer:

- 10.1.5 allows, "with user consent and after initial download of the
  primary product", the acquisition of "add-ons or extensions … that enhance
  the functionality of the product".
- 10.2.2 forbids (a) fundamentally changing or extending the **described**
  functionality through any form of dynamic inclusion of code, and (b)
  introducing that way features that violate the policies.

**Decision**: plugins are described in the listing, limited to a closed list
of extension points, installed and approved by the user, with no native
code. The listing line and the certification notes are part of the release
checklist for the first Store build with plugins. No new MSIX capability is
needed.

Under MSIX, new files under `%LOCALAPPDATA%` go to
`…\Packages\<family>\LocalCache\Local\Bientot\`, and reads fall back to the
real folder. Hence: the in-app installer is the only supported install path,
the "open folder" action shows the real path (`GetFinalPathNameByHandleW`,
already in `bientot-windows/src/diagnostics/native.rs`), and the Store
uninstall deletes plugins while Velopack keeps the data folder.

## R9. Review record

The full design (French) went through four review rounds by five reviewers
until all approved. Its findings that matter to this step are folded into
the spec, the plan and the contracts. The later steps it describes are not
specified here.

## R10. Manifest format

Measured for T003 on `x86_64-pc-windows-msvc` with the release profile: a
program that already links `serde` (derive), `serde_json` and `sha2` and reads
a manifest is 307 200 bytes; reading the same manifest with `toml` 1.x
(`default-features = false`, features `std`, `parse`, `serde`) makes it
506 880 bytes, so `toml` costs 195 KiB, above the 100 KB threshold.

**Decision**: the manifest is `plugin.json`, read with `serde_json`, which is
already in the executable. Authors get a `$schema` key for editor checks.
