# Quickstart: build, pack and install the sample plugin

This is the path tasks T040–T045 make true. Commands run from the repository
root.

## 1. Build the sample plugin

```sh
rustup target add wasm32-unknown-unknown
cargo build -p bientot-plugin-sdk --example focus --release --target wasm32-unknown-unknown
```

`.cargo/config.toml` links every `wasm32-unknown-unknown` module with a
64 KiB stack. The module is
`target/wasm32-unknown-unknown/release/examples/focus.wasm`.

## 2. Test it without Windows

```sh
cargo test -p bientot-plugin-sdk --example focus
```

The SDK's native test host runs the plugin's handlers against the day
snapshots of every demo scenario with an injected clock, and compares the
cards with the reference files in
`crates/bientot-plugin-sdk/examples/focus/expected/` (`BIENTOT_BLESS=1`
rewrites them). The snapshots are in `crates/bientot-plugin-sdk/demo-days.json`,
which a test of `bientot-plugins` builds from `DemoScenario::ALL` and keeps
in step with the host.

## 3. Pack and sign it

```sh
scripts/pack-plugin.sh crates/bientot-plugin-sdk/examples/focus \
    target/wasm32-unknown-unknown/release/examples/focus.wasm \
    --out target/focus.bientot-plugin
scripts/sign-plugin.sh target/focus.bientot-plugin --new-key ~/.bientot/plugin.key
cargo run -p bientot-plugin-tool -- check target/focus.bientot-plugin \
    --expect crates/bientot-plugin-sdk/examples/focus/expected
```

`pack-plugin.sh` validates the manifest, checks that every import of the
module is allowed by a declared capability (it names the import and the
capability to add otherwise), and writes the package with the entry names of
the package contract. Without `sign-plugin.sh`, the package is unsigned.
`--new-key` creates the key on the first signature; later signatures use
`--key`. `check` installs the package in the host's own runtime, runs it on
every demo day and compares its cards with the reference files.
`scripts/check-sample-plugins.sh` does all three for every sample, as CI
does.

## 4. Install it in a demo run

```powershell
worktime.exe --demo
```

Open **Settings › Plugins › Install from file…**, pick
`target\focus.bientot-plugin`, check the consent dialog, install, then open
the flyout: the « Focus » card shows after the counted time. Ctrl+Maj+P,
type `focus`, run « Focus: Show today or this week ».

## 5. Check the gates

```powershell
scripts/measure-performance.ps1 -Executable target\release\worktime.exe
cargo run -p bientot-plugins --example hostile-corpus --release
```

The first compares startup, working set and CPU with the release budgets,
with and without the sample plugin. The second runs every hostile plugin and
fails on any process abort.
