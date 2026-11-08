---

description: "Task list for the plugin runtime, first step"
---

# Tasks: Plugin runtime (first step)

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`

**Tests**: required. The constitution asks for tests proportional to
behavioural risk, and this step adds third-party code.

Each task names its files. **[P]** tasks touch different files and can run in
parallel. Each pull request follows `CONTRIBUTING.md`: `feat(app)` or
`test(app)` titles, a release summary for `feat(app)`, French and English
texts, and the design-system artifacts updated with any UI change.

## Phase 1: Setup

- [x] T001 Add `crates/bientot-plugin-api`, `crates/bientot-plugins` and `crates/bientot-plugin-sdk` to the workspace with empty `lib.rs` files and `publish = false`.
- [x] T002 Add the `plugins` feature to `crates/bientot-app/Cargo.toml`, on in `release` and `direct`, pulling `bientot-plugins`; check that a build without it is byte-identical in behaviour (`cargo test -p bientot-app` with and without).
- [x] T003 Measure the size cost of `toml` in the `release` executable; keep TOML if it is at most 100 KB, otherwise switch the manifest contract to JSON (`contracts/manifest.md`, `plan.md`, `quickstart.md`). Done: 195 KiB, the manifest is `plugin.json`.
- [x] T004 [P] Add a size gate for the `direct` build to the `Rust on Windows` job in `.github/workflows/ci.yml`, beside the existing `release` gate, and record both sizes in the job summary.
- [x] T005 [P] Pin `wasmi` with `default-features = false, features = ["std", "validate", "portable-dispatch"]` in `crates/bientot-plugins/Cargo.toml`; add it to `osv-scanner.toml` tracking and to the allowed licenses check of `bientot-notices`.

## Phase 2: Foundational (blocks every story)

- [x] T010 [P] Write `crates/bientot-plugin-api/wit/bientot.wit` from `contracts/host-abi.md`.
- [x] T011 [P] Implement `PluginId`, `Capability`, localized texts and `Manifest` parsing and validation in `crates/bientot-plugin-api/src/{lib,manifest}.rs`, with tests for every rule and error message of `contracts/manifest.md`.
- [x] T012 [P] Implement `DaySnapshot` v1 in `crates/bientot-plugin-api/src/snapshot.rs` and events in `src/event.rs`, every enum with `Unknown`, with JSON fixtures in `crates/bientot-plugin-api/tests/fixtures/api-1.0/` that later versions must keep reading.
- [x] T013 [P] Implement the view tree and its validation in `crates/bientot-plugin-api/src/view.rs`, with tests for each limit of `contracts/view-tree.md`.
- [x] T014 Implement the package reader in `crates/bientot-plugins/src/{zip,package,files}.rs` per `contracts/package.md`: central directory and local header cross-check, every refusal and cap, `flate2` inflate, CRC check, temporary folder and rename; tests with a crafted package per refusal.
- [x] T015 Implement signatures and trust in `crates/bientot-plugins/src/{package,trust}.rs`: canonical file list, ed25519 verification, key fingerprint, binding on first install, key announcement, unsigned hash binding; tests for each case of « Trust on install and update ».
- [x] T016 Implement `plugins/state.json` in `crates/bientot-plugins/src/state.rs` with the states and transitions of `data-model.md`, written through a temporary file and a rename; tests for each transition.
- [x] T017 Implement the runtime in `crates/bientot-plugins/src/runtime.rs`: engine, one linker with the four interfaces, import check against granted capabilities at instantiation, store limits with `memory_min` reservation and the 32 MiB pool, `call_resumable` in 2 M slices, the fuel allowance of FR-013, the wall-clock safety net, discard after trap.
- [x] T018 Implement budgets in `crates/bientot-plugins/src/budget.rs`: per-plugin and global token buckets of FR-014, host work charged; the failure record and the three-traps rule in `state.rs` and `manager.rs`; tests with an injected monotonic clock.
- [x] T019 Implement the `plugins` thread in `crates/bientot-plugins/src/thread.rs`: bounded queue, user-first priorities, `day.refreshed` coalescing, staggered first activations, eviction after 5 minutes, re-creation above max(2 ×, 1 MiB); tests with a fake runtime.
- [x] T020 Implement `bt:log@1` and the per-plugin log files under `logs\plugins\` in `crates/bientot-plugins/src/{host,log}.rs` (1 MiB, 14 days, 60 lines per minute, text field kept by the redaction); add a test in `crates/bientot-core/tests/diagnostics_export.rs` that the export holds no plugin line.
- [x] T021 [P] Implement `bt:time@1 now` and `bt:day@1 current` in `crates/bientot-plugins/src/host.rs`, and the snapshot builder from the daily summary in `crates/bientot-plugins/src/snapshot.rs`, including the departure at the minimum through `project_departure`; tests on the demo scenarios.
- [x] T022 Implement the structural-change detector for `day.refreshed` (FR-032) in `crates/bientot-plugins/src/snapshot.rs`; tests that a counted time that only grows does not fire it and that a departure moved by one minute does.

**Checkpoint**: a plugin can be installed, loaded, called and stopped from tests on Linux.

## Phase 3: User Story 1 — Install a plugin and see its card (P1) 🎯 MVP

**Independent test**: spec US1.

- [x] T030 [US1] Implement `bt:ui@1 set` and `clear` for the slot `flyout.card` in `crates/bientot-plugins/src/host.rs`, producing validated trees in the contribution updates.
- [ ] T031 [US1] Add the `PluginContributions` global and the GPUI bridge (channels, `cx.spawn`, `Arc` swap, notify) in `crates/bientot-app/src/plugins.rs`.
- [ ] T032 [US1] Convert validated trees into prepared cards with the app's widgets and tokens in `crates/bientot-app/src/plugins/card.rs`; `duration` and `countdown` read the clock at render.
- [ ] T033 [US1] Add `Part::PluginCard` after `Part::Worked` in `crates/bientot-app/src/flyout/body.rs`: one visible card, others under **More**, header with icon, name and « … » menu (**Hide for today**, **Plugin settings**, **Turn off**), focus restored, no live-region announcement; store `hidden_on` like the closed callouts.
- [ ] T034 [US1] Add `Page::Plugins` to `crates/bientot-app/src/settings.rs` and `crates/bientot-app/src/settings_view/plugins.rs`: list, states of FR-044, turn on or off, remove, **Install from file…**, drop target, **Open folder** at its real path through `bientot-windows`.
- [ ] T035 [US1] Add the consent dialog (capabilities in plain words, fingerprint or unsigned warning, declared memory) in `crates/bientot-app/src/settings_view/plugins.rs`.
- [ ] T036 [US1] Add every new text in French and English to `crates/bientot-app/src/strings/table.rs`; consent and security texts can never come from a plugin.
- [ ] T037 [US1] Make the transfer file carry only plugin ids and versions in `crates/bientot-store/src/transfer.rs`; test that an export of a data folder with plugins holds no plugin file or state.
- [ ] T038 [US1] Add screenshots of the card, the consent dialog and **Settings › Plugins** to `crates/bientot-app/src/bin/screenshots/` (FR/EN, light/dark), and update the design system and mockup artifacts; run the design review panel.

**Checkpoint**: US1 acceptance scenarios pass on Windows.

## Phase 4: User Story 2 — Fast and never falls over (P1)

**Independent test**: spec US2.

- [x] T040 [US2] Write the hostile corpus in `crates/bientot-plugins/tests/hostile/`: `call_indirect` loop, deep recursion, `memory.grow` storm, huge `memset`, oversized and deep view trees, import call loop, log flood, invalid JSON, missing exports, start-function loop.
- [x] T041 [US2] Add the `hostile-corpus` runner as `crates/bientot-plugins/examples/hostile-corpus.rs`; it fails on an abort, on a working set more than 5 MiB above its start once instances are dropped, or on a flyout render that waited for a call.
- [x] T042 [US2] Run the corpus on the Windows release build in the `Rust on Windows` job of `.github/workflows/ci.yml`.
- [ ] T043 [US2] Add `--no-plugins` and the safe mode (marker, two failed starts, flyout notice with **Turn plugins back on**) in `crates/bientot-app/src/launch.rs` and `crates/bientot-app/src/main.rs`, with tests in `launch.rs`.
- [ ] T044 [US2] Check FR-018 and FR-019 with a test in `crates/bientot-app/src/host/tests.rs`: no thread, no engine and no manifest read before `startup.tray_ready`; with no plugin enabled, none at all.
- [ ] T045 [US2] Extend `scripts/measure-performance.ps1` to measure with no plugin and with the sample plugin, and report SC-002 and SC-003 in the job summary.

**Checkpoint**: SC-001 to SC-004 measured on the CI Windows runner.

## Phase 5: User Story 3 — Plugin commands (P2)

**Independent test**: spec US3.

- [ ] T050 [US3] Add `PaletteRow` (`Builtin(Command)` or `Plugin { plugin, command }`) in `crates/bientot-app/src/palette.rs` and `palette_view.rs`: listing as « <plugin>: <title> » from manifests, filtering, running through the plugin thread with user priority.
- [ ] T051 [US3] Route card buttons to `command.invoked` with `arg` and `source: "card"` in `crates/bientot-app/src/plugins/card.rs`.
- [ ] T052 [US3] Refuse built-in titles and `shortcut` keys at install, with the messages of `contracts/manifest.md`; tests in `crates/bientot-plugin-api/src/manifest.rs`.

## Phase 6: User Story 4 — A bad plugin is stopped (P2)

**Independent test**: spec US4.

- [ ] T053 [US4] Implement the revocation list (verification with the built-in key, version and expiry checks, application to `state.json`, last good list) in `crates/bientot-plugins/src/revocation.rs`; tests per rule of `contracts/revocations.md`.
- [ ] T054 [US4] Fetch the list with the update check and before an install or a turn-on, on a background thread, in `crates/bientot-app/src/plugins.rs`; skip in debug and demo runs; tell the user when the last list was used.
- [ ] T055 [P] [US4] Serve `/plugins/revocations.json` as a static file from `services/bientot` (`site` scope pull request), with the first empty list signed by the maintainer's offline key.
- [ ] T056 [US4] Check file hashes at every load and show `FilesChanged` with **Remove** in **Settings › Plugins**; test by editing a file of an installed plugin.

## Phase 7: User Story 5 — Build the sample plugin (P3)

**Independent test**: spec US5 and `quickstart.md`.

- [x] T057 [P] [US5] Implement the SDK in `crates/bientot-plugin-sdk`: exports, buffer handling, generated types, view builders, a panic hook that logs the message before the trap, the 64 KiB stack link setting, and a native test host that runs handlers on demo snapshots with an injected clock.
- [x] T058 [US5] Write the sample « Focus » plugin in `crates/bientot-plugin-sdk/examples/focus/` with its manifest, FR/EN locales and reference cards (the manifest contract has no icon). Three more samples: `tomato`, `weekend`, `battery`.
- [x] T059 [US5] Write `scripts/pack-plugin.sh` and `scripts/sign-plugin.sh` (validation, import check naming the missing capability, canonical list, key creation).

## Phase 8: Polish and cross-cutting

- [x] T060 Confirm the timings of `research.md` and the `auto-dispatch` abort on a real Windows machine; update `research.md`.
- [x] T061 Measure the size cost of `ed25519-dalek` and record it in `research.md`.
- [ ] T062 [P] Add the fuzz targets `manifest`, `package`, `host-call` and `view-tree` to `fuzz/`, with seeds from the tests, and to `.github/workflows/fuzz.yml`.
- [ ] T063 [P] Update `docs/privacy.md` (the revocation request, `plugins\`, `logs\plugins\`, the transfer file), `docs/architecture.md` (a « Plugins » section and the new crates), `SECURITY.md` (host and SDK in scope) and `docs/code-signing.md` (the offline keys).
- [ ] T064 [P] Add to `docs/release-checklist.md` the Store listing line and certification notes for the first Store build with plugins (`research.md` R8).
- [ ] T065 Run the full validation of `quickstart.md` on Windows and record it in `docs/validation-report.md`.

## Dependencies

- Phase 2 depends on Phase 1. Every story depends on Phase 2.
- US1 (Phase 3) and US2 (Phase 4) can be built in parallel once Phase 2 is
  done; US2's measurements need T058's sample plugin.
- US3 depends on T031 and T032. US4 depends on T016. US5 can start right
  after T010–T013.
- T055 is a separate `site` pull request and must be deployed before T054
  ships.

## Suggested pull requests

1. Phase 1 and T010–T013 (`feat(app)`, no user-visible change, no release
   summary needed beyond « no visible change »).
2. T014–T022 (host, tests on Linux).
3. T057–T059 and T040–T042 (SDK, sample, hostile corpus in CI).
4. Phase 3 (the first visible change).
5. Phase 4 remainder, Phase 5 and Phase 6.
6. Phase 8.
