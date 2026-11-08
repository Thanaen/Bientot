---
name: write-a-bientot-plugin
description: Write, test, pack and sign a plugin for Bientôt, the Windows tray app that tracks working time and tells you when you can leave. Plugins are Rust built to WebAssembly with bientot-plugin-sdk and show one card in the flyout. Use when creating or changing a Bientôt plugin (plugin.json, locales, the card, commands, tests), or when its build, tests, pack or check fail.
license: MIT
compatibility: Needs Rust 1.97 or later with the wasm32-unknown-unknown target, and network access to github.com for the SDK and the tool.
metadata:
  api: "1.0"
  source: https://github.com/Thanaen/Bientot/tree/main/crates/bientot-plugin-sdk
---

# Write a Bientôt plugin

A Bientôt plugin is a small WebAssembly module, a manifest and its texts. Bientôt runs it in a
sandbox and gives it the day (counted time, target, departures, segments). The plugin shows one
card in the flyout, and the user runs its commands from the card or the command palette. A
plugin never draws: it describes a card, and Bientôt draws it in the user's language and theme.

The reference is the SDK's README:
https://github.com/Thanaen/Bientot/blob/main/crates/bientot-plugin-sdk/README.md. When this skill
and the README disagree, the README wins. The four samples in `crates/bientot-plugin-sdk/examples/`
(`focus`, `tomato`, `weekend`, `battery`) are complete plugins to copy from.

## What api 1.0 can and cannot do

It can read the day (`read:day`), show one card (`ui:flyout`), write log lines, read the clock,
and offer up to 8 commands. It cannot reach the network, store files, read settings, set timers,
change the day or run in the background on its own. If the user asks for one of those, say that
api 1.0 does not offer it, and propose what fits instead. For example, a timer whose countdown
Bientôt draws, with its state carried in a button's `arg`, as the `tomato` sample does. Never
work around the sandbox.

## Steps

### 1. Set up once

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked --git https://github.com/Thanaen/Bientot bientot-plugin-tool
```

The second command installs `bientot-plugin`, which packs, signs and checks plugins.

### 2. Create the five files

```text
my-plugin/
├── Cargo.toml
├── .cargo/config.toml
├── plugin.json
├── locales/en.json        # one file per locale of plugin.json
└── src/lib.rs
```

`Cargo.toml`:

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

A module of about 400 KiB is normal: most of it is `serde_json` and `jiff`, which the SDK uses.

`.cargo/config.toml` keeps the module small. Without it, Rust reserves a 1 MiB stack and every
instance starts above the memory Bientôt reserves:

```toml
[target.wasm32-unknown-unknown]
rustflags = ["-C", "link-arg=-zstack-size=65536"]
```

`plugin.json`:

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

Manifest rules that are easy to miss:

- `id` is dotted lower case, like `me.hello`: letters and digits, then parts that may also hold
  `-`; at most 64 characters; `dev.` is reserved. Never change it after a release.
- Every localized text needs the first locale of `locales`, which is the fallback.
- `name` has at most 40 characters, `description` 120, and a command `title` 48. A title may not
  equal one of Bientôt's own commands.
- A command `name` matches `^[a-z][a-z0-9-]{0,31}$`. There is no `shortcut` key.
- `day.refreshed` needs `read:day`. `installed`, `upgraded`, `activate` and `command.invoked`
  always arrive and are not listed in `events`.
- `memory` defaults to `8MiB`, at most `32MiB`; `2MiB` is plenty for most plugins.
- Unknown keys are refused, as are the keys of later api versions (`settings`, `storage`, `net`…).

`locales/<tag>.json` is a flat object of keys to texts. `{name}` is replaced by the argument
`name`, and nothing else is interpreted. Write French as Bientôt does: a no-break space
(U+00A0) before `:`, `?`, `!` and `%`, and typographic apostrophes (’).

`locales/en.json`:

```json
{ "counted": "Counted today", "waves": "Waves: {count}", "wave": "Wave" }
```

`locales/fr.json`, with a no-break space before the colon:

```json
{ "counted": "Compté aujourd’hui", "waves": "Saluts : {count}", "wave": "Saluer" }
```

### 3. Write the plugin

`src/lib.rs`:

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

    fn command(&mut self, command: &str, _arg: Option<&str>, _source: CommandSource) {
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

Design the plugin around these rules:

- **An instance can be dropped between two events**: after five idle minutes, after a trap, or
  when its memory doubles. api 1.0 has no storage, so keep nothing in memory that you cannot
  rebuild from the day. A button's `arg` (`button("go", label).arg(state)`, any `ToString`)
  comes back as `command`'s `arg` when the user presses that button, and only then:
  `day_refreshed` gets no `arg`. So an `arg` carries state from one press to the next, as the
  `tomato` timer does. State that the next refresh needs, like a snooze, lives in memory and
  is lost with the instance. Design the plugin so that losing it is harmless (the reminder
  comes back early), and tell the user that limit. `waves` above resets with the instance,
  which is fine for a demo and wrong for a counter that matters.
- **Let Bientôt draw what moves.** `day.refreshed` comes only when the day changes: a segment
  settles, the tracking state or the target changes, or a departure moves by a minute or more.
  It comes at most every 30 s. Time passing alone sends nothing, so a card cannot change shape
  at a set time. Show what depends on the clock with `duration_since` (grows) and `countdown`
  (runs down): Bientôt redraws them without waking the plugin. For a "time for a break" alert,
  show a countdown to the moment rather than a badge that should appear then.
- **A card** has at most 4 lines and 2 buttons, 64 nodes and 6 levels, and each text is at
  most 200 characters once filled. A progress bar needs `0 ≤ value ≤ max` and `max > 0`: clamp
  the value when the counted time passes the target, and leave the bar out when the target is
  0 (non-working days). Never put a colour, a size or raw text in a card: every text is a key
  of the locale files.
- **At most two `ui::show` per event.** It returns `Err(reason)` when Bientôt refuses the card.
  Bientôt then keeps the previous card and counts a failure, and three failures in ten minutes
  turn the plugin off until Bientôt restarts.
- **Match the enums with a `_` arm**: later versions add variants.
- **Budgets:** each event gets 20 M fuel, plus 250 per byte of the event (100 M at most; 400 M
  for a command). Allocating in a loop or parsing JSON by hand is what spends it. The samples
  use at most 1.7 M. Log at most 60 lines a minute with `log::info`, `log::warn` and
  `log::error`.
- A module imports only the calls it uses, and Bientôt refuses a module that uses a call its
  manifest does not grant: `day::current` needs `read:day`, `ui::show` and `ui::clear` need
  `ui:flyout`.

### The API in one place

`use bientot_plugin_sdk::prelude::*;` brings all of it. Instants are `Timestamp` and spans are
`SignedDuration`, both from `jiff`. Use `t.as_second()`, `Timestamp::from_second(s)` (a `Result`),
`t + SignedDuration::from_mins(30)` and `later.duration_since(earlier)`; an instant is not an
`i64`.

| Call | Returns |
| --- | --- |
| `time::now()` | `Timestamp`, the current instant |
| `day::current()` | `Option<DaySnapshot>`, `None` before Bientôt's first calculation |
| `ui::show(card)` | `Result<(), String>`, the reason when Bientôt refuses the card |
| `ui::clear()`, `log::info(&str)`, `log::warn(&str)`, `log::error(&str)` | `()` |

The card builders:

| Builder | Draws |
| --- | --- |
| `card().line(node).line_if(Option<node>).action(button)` | the card: lines, then buttons |
| `row![a, b]`, `stack![a, b]` | children side by side, or one under the other |
| `text(key).arg(name, value)` | the text of `key`, `{name}` filled with `value` |
| `duration(seconds: i64)` | a duration in the app's format |
| `duration_since(seconds: i64, since: Timestamp)` | `seconds` plus the time since `since`, growing |
| `countdown(until: Timestamp)` | the time left until `until`, running down |
| `time_at(at: Timestamp)` | a time of day, in the user's zone |
| `badge(text(key), Tone::Warning)` | a badge; tones `Info`, `Success`, `Warning`, `Error`, `Muted` |
| `progress(value: f64, max: f64, label: text(key))` | a progress bar; the label is required |
| `button(command: &str, label: text(key)).arg(state)` | a button that runs a manifest command |

The text roles `.title()`, `.muted()`, `.value()` and `.mono()` are methods of `text(..)` only.
A duration, a countdown or a row has no role.

The day, `DaySnapshot`:

| Field | Holds |
| --- | --- |
| `computed_at: Timestamp` | when Bientôt calculated the day |
| `date`, `time_zone: String` | the local date and the IANA zone, like `Europe/Paris` |
| `language` | `Language::En` or `Language::Fr` |
| `tracking` | `Running`, `Paused`, `Finished`, `NonWorkingDay` |
| `counted`, `target`, `minimum: i64` | seconds; `target` can be 0, as on a non-working day |
| `departure_at_target`, `departure_at_minimum: Option<Timestamp>` | `None` once reached or without a forecast |
| `forecast` | `Firm`, `Estimated`, `Unavailable` |
| `segments: Vec<Segment>` | settled segments, oldest first: `kind`, `start`, `end` |
| `open_tail: Option<OpenTail>` | the segment under way: `kind`, `start` |
| `week.credit: i64` | seconds, positive when ahead |

`SegmentKind` is `Work`, `Break`, `Lunch`, `Away`, `Manual`, `Appointment`, `Other` or
`Unknown`. `kind.is_work()` is true for `Work`, `Manual` and `Appointment`. A day with activity
starts with an `Away` segment from midnight to the arrival, and a lunch can come as `Away`. So
skip what comes before the first work when you look for breaks. When Bientôt cannot see
activity, `segments` is empty and the open tail is `Other`, from midnight.

### 4. Test it without Windows

The SDK's `testing::TestHost` answers the calls in a native build as Bientôt does. It checks
cards against the manifest and the locale files, and it checks capabilities. The test sets the
clock and the day. Put the tests at the end of `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bientot_plugin_sdk::testing::{TestHost, assert_reference, demo_day, demo_report};

    #[test]
    fn counts_the_waves() {
        let mut host = TestHost::load(env!("CARGO_MANIFEST_DIR"));
        let mut plugin = Hello::default();
        host.refresh(&mut plugin, demo_day("OnTrack")).unwrap();
        host.press(&mut plugin, 0).unwrap();
        assert!(host.render().contains("Waves: 1"));
    }

    #[test]
    fn every_demo_day() {
        for (language, tag) in [(Language::En, "en"), (Language::Fr, "fr")] {
            let report = demo_report::<Hello>(env!("CARGO_MANIFEST_DIR"), language);
            assert_reference(
                concat!(env!("CARGO_MANIFEST_DIR"), "/expected/").to_owned() + tag + ".txt",
                &report,
            );
        }
    }
}
```

`demo_report` runs the plugin on the 13 demo days of Bientôt (`OnTrack`, `Late`, `Complete`,
`Morning`, `Unavailable`, `Credit`, `FragmentedDay`…). Then it presses the first button of the
last day's card (`ActivityWatchUnavailable`) up to three times, 30 minutes apart. If that card
has no button, the report has no presses, so test the commands with `TestHost` yourself.
Create or update the reference files with `BIENTOT_BLESS=1 cargo test`, then read every
card in `expected/*.txt` as a user would: a missing text, a wrong unit or an empty line is a bug
to fix in the plugin. Run plain `cargo test` afterwards; it must pass.

The demo days are in UTC, calculated at 11:00, 16:00 or 18:00. `host.refresh(&mut plugin, day)`
sets the clock to `day.computed_at`, which undoes an earlier `host.at(..)` or
`host.advance(..)`. To test a later moment, call `host.advance(SignedDuration::from_mins(61))`
after the refresh and before the next event, or move `day.computed_at` before the refresh.

```rust
let mut day = demo_day("OnTrack");
host.refresh(&mut plugin, day.clone()).unwrap();
host.press(&mut plugin, 0).unwrap();                    // the first button of the card
host.command(&mut plugin, "wave", None).unwrap();       // from the palette: no arg
day.computed_at = day.computed_at + SignedDuration::from_mins(61);
host.refresh(&mut plugin, day).unwrap();                // an hour later
```

Use `host.language(Language::Fr)` for French and `host.logs()` to see the log lines. A test
that calls `day::current` without `read:day` panics with the capability to add.

### 5. Build, pack, sign and check

```sh
cargo build --release --target wasm32-unknown-unknown
bientot-plugin pack . target/wasm32-unknown-unknown/release/my_plugin.wasm --out hello.bientot-plugin
bientot-plugin keygen ~/.bientot/plugin.key        # once; keep the key file safe
bientot-plugin sign hello.bientot-plugin --key ~/.bientot/plugin.key
bientot-plugin check hello.bientot-plugin --expect expected
```

- `pack` makes the install checks that need no app. It names the capability a module lacks and
  the build mistake behind a missing export or a WASI import.
- `sign` binds later versions to the key. An update installs only when the same key signs it, or
  a key it announced: `--announce <new public key>` on the last version signed with the old key,
  or `--handover <old key file>` when signing with the new one. A lost key means a new plugin id.
  Never commit the key file.
- `check` installs the package in Bientôt's own runtime and runs it over the demo days. It prints
  the cards, the fuel used and the memory. With `--expect`, it compares the cards with the
  reference files, which must still match.

An unsigned package installs too, with a warning in the consent dialog.

## When something fails

| Symptom | Fix |
| --- | --- |
| `error[E0463]: can't find crate for core` | `rustup target add wasm32-unknown-unknown` |
| pack: an import needs a capability | add it to `capabilities` in `plugin.json` |
| pack: missing `bt_handle` or `bt_alloc` | call `export!` once, and set `crate-type = ["cdylib"]` |
| pack: unknown import `wasi_…` | build for `wasm32-unknown-unknown`, not `wasm32-wasip1` |
| check: memory above the manifest | add `.cargo/config.toml` with the 64 KiB stack |
| a refused card: unknown key | add the key to every locale file, or fix its spelling |
| a refused card: too many lines, nodes or levels | at most 4 lines, 2 buttons, 64 nodes, 6 levels |
| a refused card: unknown command | the button's command must be in the manifest's `commands` |
| the reference file differs | read the diff; if the new cards are right, `BIENTOT_BLESS=1 cargo test` |
| `the call used up its fuel` | the code does too much per event: cache less, allocate less, keep loops bounded |

## Before handing the plugin over

- `cargo test` passes and the reference files were read, in every locale.
- `cargo clippy --target wasm32-unknown-unknown -- -D warnings` is clean.
- `bientot-plugin check` matches the references and reports fuel well under 20 M.
- Every text exists in every locale of `plugin.json`, and the French has its no-break spaces.
- The key file is outside the repository.
