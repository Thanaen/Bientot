# Bientôt

Bientôt is a small Windows app that lives in the notification area and
answers one question:

> Am I still on track to finish today's work before the time I want to leave?

It is free, open source (MIT), local-first, and maintained by one person in
their free time. The app is available in French and in English.

Bientôt was called WorkTime until October 2026. Copies installed under the
old name update in place and keep their data.

## What it does

The tray icon changes color with your day:

- **green**: comfortably on track;
- **orange**: your projected departure is close to your latest departure time;
- **red**: your projected departure is later than your latest departure time;
- **blue**: today's target is reached, and overtime is accumulating.

Click it to open a compact view with the time worked today, the time left,
and your projected departure time. From there and from the main window you can:

- pause, resume or finish your day;
- declare time worked away from the computer, or correct a recorded period;
- see your week, with the credit carried from one day to the next;
- get a notification shortly before you reach your target or your latest
  departure time;
- set recurring appointments, such as a weekly meeting away from your desk;
- export your data to a `.bientot` file and import it on another PC.

Bientôt detects activity on its own. It checks when your keyboard or mouse
was last used, and after 3 minutes without input (adjustable from 30 seconds
to 30 minutes) it counts the time as idle. It never reads what you type,
application names or window titles.

## Download

Bientôt runs on Windows 11 x64.

- **Installer**: download `Setup.exe` from
  [bientot.dev/download](https://bientot.dev/download). The
  app then updates itself. The installer is not signed yet, so Windows
  SmartScreen may ask you to confirm with **More info › Run anyway**.
- **Microsoft Store**: planned for version 1.0.

Bientôt is still in alpha: expect rough edges, and please
[report what goes wrong](https://github.com/Thanaen/WorkTime/issues/new/choose).

## Privacy

Your working time never leaves your computer. Bientôt stores its data in
`%LOCALAPPDATA%\Bientot` and has no account and no cloud sync. It makes two
kinds of network request:

- a daily update check, for the installer and Store builds;
- one anonymous usage signal a day (app version, install channel, and whether
  it is the first signal of the month), with no identifier. It is shown during
  setup and you can turn it off in Settings.

[docs/privacy.md](docs/privacy.md) lists every stored file and every request,
and explains how to delete your data.

## Default settings

| Setting | Default |
| --- | --- |
| Daily target | 7 hours |
| Usual lunch window | 12:00–14:00 |
| Latest departure | 18:30 |
| Orange warning margin | 15 minutes |
| Minimum work after applying carried credit | 6 hours |
| Idle threshold | 3 minutes |
| Start with Windows | on, and offered during setup |

Everything is adjustable in Settings.

## Build from source

The app is written in Rust with the [GPUI](https://www.gpui.rs/) interface
toolkit, in the `crates/` workspace. The toolchain pinned in
`rust-toolchain.toml` installs on the first `cargo` command.

```bash
cargo test --workspace --all-features --locked
```

On Windows, run the app with scripted activity and temporary data:

```powershell
cargo run -p bientot-app --bin worktime -- --demo --french --dark
```

`--english` and `--light` switch the language and theme. The small web
service behind `bientot.dev` lives in `services/worktime` (Bun).

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full checks, localization, and
how to propose a change, and [docs/architecture.md](docs/architecture.md) for
the code layout.

## Contributing

Bug reports, ideas and pull requests are welcome, in English or in French.
Please open an issue before starting on something large. Report security
problems privately, as described in [SECURITY.md](SECURITY.md). Everyone
taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## License

Bientôt is released under the [MIT license](LICENSE). The app embeds the
[Lucide](https://lucide.dev) icons (ISC,
[notice](crates/bientot-app/assets/icons/LICENSE)) and the IBM Plex fonts
(SIL Open Font License, [notice](crates/bientot-app/assets/fonts/OFL.txt)).
The downloads also ship `THIRD-PARTY-NOTICES.txt`, which lists the Rust crates
compiled into `worktime.exe` with their licenses. `scripts/package-rust.ps1`
writes it with `cargo run --locked -p bientot-notices -- <file>`, which fails
on a license outside the allowed list.
