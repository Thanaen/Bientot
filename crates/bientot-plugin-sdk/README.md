# Writing a Bientôt plugin

A Bientôt plugin is a small WebAssembly module with a manifest and its texts. Bientôt runs it in
a sandbox, on its own thread. The plugin reads the day (counted time, target, departures,
segments) and shows one card in the flyout. Users run its commands from the card or the command
palette. A plugin never draws pixels: it describes a card, and Bientôt draws it with its own
components, in the user's language and theme.

This crate is the Rust side of api 1.0. The contracts it implements are in
[`specs/003-plugin-runtime/contracts/`](../../specs/003-plugin-runtime/contracts/).

## A plugin in five files

Plugins build for `wasm32-unknown-unknown`: `rustup target add wasm32-unknown-unknown` once.

```text
my-plugin/
├── Cargo.toml
├── .cargo/config.toml
├── plugin.json
├── locales/en.json
└── src/lib.rs
```

`Cargo.toml` builds a `cdylib`:

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
bientot-plugin-sdk = { git = "https://github.com/Thanaen/Bientot" }

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
strip = true
```

`.cargo/config.toml` links the module with a 64 KiB stack. Without it, Rust reserves 1 MiB, and
every instance starts above the default memory reservation:

```toml
[target.wasm32-unknown-unknown]
rustflags = ["-C", "link-arg=-zstack-size=65536"]
```

`plugin.json` says who the plugin is and what it may do (contract
[`manifest.md`](../../specs/003-plugin-runtime/contracts/manifest.md)):

```json
{
  "id": "me.hello",
  "name": { "en": "Hello", "fr": "Bonjour" },
  "version": "0.1.0",
  "api": "1.0",
  "memory": "2MiB",
  "locales": ["en", "fr"],
  "capabilities": ["read:day", "ui:flyout"],
  "events": ["day.refreshed"],
  "commands": [{ "name": "wave", "title": { "en": "Wave", "fr": "Saluer" } }]
}
```

`locales/<tag>.json` holds the texts of the card, as a flat object. `{name}` is replaced by an
argument of the same name:

```json
{ "counted": "Counted today", "waves": "Waves: {count}", "wave": "Wave" }
```

`src/lib.rs` implements `Plugin` and exports it:

```rust
use bientot_plugin_sdk::prelude::*;

#[derive(Default)]
struct Hello {
    waves: u32,
}

impl Hello {
    fn show(&self, day: &DaySnapshot) {
        let _ = ui::show(
            card()
                .line(row![text("counted"), duration(day.counted)])
                .line(text("waves").arg("count", self.waves).muted())
                .action(button("wave", text("wave"))),
        );
    }
}

impl Plugin for Hello {
    fn day_refreshed(&mut self, day: &DaySnapshot) {
        self.show(day);
    }

    fn command(&mut self, command: &str, _: Option<&str>, _: CommandSource) {
        if command == "wave" {
            self.waves += 1;
            if let Some(day) = day::current() {
                self.show(&day);
            }
        }
    }
}

bientot_plugin_sdk::export!(Hello);
```

## What a plugin receives

| Method | When |
| --- | --- |
| `installed`, `upgraded` | once, after an install or an update |
| `activate` | each time an instance starts: at Bientôt's start, after five idle minutes, or after a trap |
| `day_refreshed` | when the day changes in a way worth a look, at most every 30 s (needs `read:day` and the event in `events`) |
| `command` | the user ran a command of the manifest, from the palette or a card button |
| `language_changed` | the app language changed (needs the event in `events`) |

An instance can be dropped between two events, so keep nothing in memory you cannot rebuild from
the day. A button's `arg` comes back with the command: the sample « Tomato » keeps its timer
there.

## What a plugin can call

| Call | Needs |
| --- | --- |
| `log::info`, `log::warn`, `log::error` | nothing; 60 lines a minute, in `logs\plugins\<id>.jsonl` |
| `time::now()` | nothing |
| `day::current()` | `read:day` |
| `ui::show(card)`, `ui::clear()` | `ui:flyout` |

A module imports only the calls it uses, and Bientôt refuses a module that uses a call its
manifest does not grant.

## Cards

A card has at most 4 lines and 2 buttons, 64 nodes and 6 levels. Each text is at most 200
characters once filled. The builders are `card`, `row!`, `stack!`, `text`, `duration`,
`duration_since` (grows with the clock, drawn by Bientôt), `countdown`, `time_at`,
`badge`, `progress` and `button`. Bientôt refuses a card that breaks a rule, keeps the
previous one, and counts a failure. `ui::show` returns the reason.

Times are drawn in the user's zone and durations in the app's format. The snapshot gives
instants in UTC, and the local date as it is (`day.date`).

## Limits

Each event may use 20 M fuel plus 250 per byte of the event (at most 100 M, 400 M for a command)
and 500 ms of CPU (2 s for a command). Over a minute, a plugin gets 20 ms of background CPU,
after a reserve of 500 ms. A plugin that traps three times in ten minutes stays off until
Bientôt restarts. The samples use at most 1.7 M fuel per event.

## Testing

In a native build, the same calls go to `testing::TestHost`. It answers them as Bientôt does:
a card is checked against the manifest and the locale files, a call needs its capability, and
the clock and the day are yours to set.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bientot_plugin_sdk::testing::{TestHost, demo_day};

    #[test]
    fn counts_the_waves() {
        let mut host = TestHost::load(env!("CARGO_MANIFEST_DIR"));
        let mut plugin = Hello::default();
        host.refresh(&mut plugin, demo_day("OnTrack")).unwrap();
        host.press(&mut plugin, 0).unwrap();
        assert!(host.render().contains("Waves: 1"));
    }
}
```

`testing::demo_days` gives the days of Bientôt's demo scenarios, built by the real accounting.
`testing::demo_report` runs the plugin over all of them and renders each card. Compare it
with reference files through `testing::assert_reference`; `BIENTOT_BLESS=1` rewrites them.

## Pack, sign, check

From a clone of the Bientôt repository:

```sh
cargo build --release --target wasm32-unknown-unknown            # in your plugin
scripts/pack-plugin.sh <plugin folder> <module.wasm> --out hello.bientot-plugin
scripts/sign-plugin.sh hello.bientot-plugin --new-key ~/.bientot/plugin.key
cargo run -p bientot-plugin-tool -- check hello.bientot-plugin
```

`pack` makes every check Bientôt makes at install that does not need the app, and names the
capability a module lacks. `sign` binds later versions to your key: an update installs only when
the same key signs it, or a key it announced (`--announce`, `--handover`), so keep the key file
safe. `check` installs the package in Bientôt's own runtime, runs it over the demo days and
prints its cards, the fuel it used and its memory. With `--expect <folder>`, it compares the
cards with your reference files.

To try the plugin in Bientôt without going through **Settings › Plugins**, close Bientôt and run
`cargo run -p bientot-plugin-tool -- install hello.bientot-plugin --data-dir <folder>`, then start
Bientôt with `--data-dir <folder>`. It installs the plugin turned on with everything it asks for,
as if you had accepted the consent dialog. An update keeps a plugin you turned off as it is.

## With Claude Code

The repository is a Claude Code plugin marketplace. Its skill `write-a-bientot-plugin` walks
an agent through these steps, from the five files to `check`:

```text
/plugin marketplace add Thanaen/Bientot
/plugin install bientot@thanaen-bientot
```

The skill is a plain `SKILL.md` in
[`plugins/bientot/skills/write-a-bientot-plugin/`](../../plugins/bientot/skills/write-a-bientot-plugin/SKILL.md):
copy that folder into `.claude/skills/` of your project to use it without the marketplace.

## Samples

| Sample | Shows |
| --- | --- |
| [`focus`](examples/focus) | the longest stretch of work and the one under way; a growing duration, a toggled detail |
| [`tomato`](examples/tomato) | a 25-minute focus timer; no `read:day`, state carried in button `arg`s |
| [`weekend`](examples/weekend) | days to Friday, then a countdown to Friday's departure, and the week's balance |
| [`battery`](examples/battery) | the day as a battery that runs down; a progress bar, badges, a coffee counter, `language.changed` |

`scripts/check-sample-plugins.sh` builds, packs and checks them all, as CI does.
