# Architecture

WorkTime is one Rust process with a [GPUI](https://www.gpui.rs/) interface.
The rules live in crates that build and test on any OS; only the Win32 calls
and the windows on screen need Windows. This page describes the code as of
October 2026.

## Crates

The Cargo workspace is `crates/*`. Each crate depends only on the crates above
it in this table.

| Crate | Builds on | Role |
| --- | --- | --- |
| `worktime-domain` | any OS | The accounting rules, as pure functions over a clock and a time zone: intervals, workday accounting, manual adjustments, weekly credit, forecasts and indicator state, daily summaries, reminders, settings validation, the usage-signal rules. |
| `worktime-store` | any OS (DPAPI on Windows) | Persistence in the data directory: `settings.json`, the SQLite ledger and its migrations, `app-state.json`, the DPAPI secrets, and the `.worktime` transfer file. |
| `worktime-core` | any OS | The app services: the activity source router, the accounting of one day, the weekly recalculation, tracking commands, corrections, long-lock reviews, reminders, the settings form logic, the demo scenarios, diagnostics, the usage signal sender. |
| `worktime-windows` | Windows for its Win32 calls | The built-in activity source (last-input time), session lock and power events, power history from the System event log, start with Windows, toast notifications, the single-instance claim, file dialogs, Microsoft Store updates. |
| `worktime-app` | Windows to run; tests on any OS | The `worktime.exe` binary: the notification-area icon (`tray-icon`), the flyout, the week, « Corriger le temps… », settings, onboarding, the command palette, the theme and the FR/EN strings. It also holds the `worktime-screenshots` tool. |
| `worktime-notices` | any OS | Writes `THIRD-PARTY-NOTICES.txt` for the release executable and fails on a license outside the allowed list. |

The web service behind `worktime.thanaen.dev` (usage counter, update redirects,
download link) is a separate Bun project in `services/worktime`; see its
[README](../services/worktime/README.md).

## Launch

`crates/worktime-app/src/main.rs` parses the command line (`launch.rs`),
resolves the data directory, then claims the single instance for that
directory (`worktime_windows::single_instance`). If another WorkTime already
holds it, the launch hands its activation over, logs `instance.redirected` and
exits before it opens the settings, the ledger or the log. A `--data-dir` or a
`--demo` launch uses another folder, so it runs on its own.

On a first run, the onboarding window opens before anything else. Its first
step can import a `.worktime` file instead (`transfer.rs`): WorkTime then
restarts, applies it before it loads the settings, and skips the onboarding,
since the file brings its settings. Otherwise
the app loads the settings and opens the day (`live.rs`): the diagnostic log,
the migrated ledger, the built-in source behind the source router, the weekly
recalculation and the tracking commands. When it cannot start, it opens a
window that says why and offers the diagnostic export
(`startup_failure_view.rs`).

## Data flow

1. **Activity.** While the built-in source is selected, `host.rs` samples the
   last keyboard or mouse input every 5 seconds. `worktime_windows::activity`
   turns the samples into active and idle spans with the configured idle
   threshold (180 s by default) and stores the settled ones. Time before
   WorkTime starts and the pending tail after the last input stay unknown. See
   the [built-in source contract](../specs/001-core-workday-indicator/contracts/built-in-activity-source.md).
2. **Routing.** `worktime_core::routing` sends each part of a read to the
   source selected at that instant. ActivityWatch, an optional source of the
   first .NET app, is no longer read: the days it covered keep their stored
   summaries.
3. **Accounting.** Every 30 seconds, and on F5 or a tracking action,
   `worktime_core::accounting` computes the day from the spans, the session
   and power events, the manual adjustments and the recurring appointments.
   `worktime_core::weekly` then recalculates the current week, stores the
   daily summaries and the weekly credit, and closes earlier weeks.
4. **Display.** The tray icon, its tooltip, the flyout and the week read the
   published state (`workday.rs`, `tray_model.rs`, `week.rs`).
   `worktime_core::notifications` decides which reminders are due, and the
   notification receipts in the ledger make each one show once.

## Where state lives

The data directory is `%LOCALAPPDATA%\WorkTime`. A `--data-dir <path>`
argument overrides it, and demo runs use a new folder under
`%TEMP%\WorkTimeDemo`. The first .NET app used the same folder and files, so
the Rust app reads its data with no conversion.

| Path | Content |
| --- | --- |
| `settings.json` | Schedule, notifications, recurring appointments, idle threshold, language, update check state, usage signal choice. Written through a temporary file and a rename. An invalid file is renamed to `settings.json.invalid-<guid>.json` and defaults are restored. |
| `worktime.db` | SQLite ledger: activity source selections, active and idle spans, session and power events, manual corrections, long-lock resolutions, daily summary revisions, credit ledger, notification receipts. |
| `app-state.json` | The date of the last usage signal the service recorded, and the flyout callouts closed today. |
| `secrets\*.secret` | Secrets encrypted with DPAPI for the current Windows user. |
| `logs\worktime-*.jsonl` | Structured diagnostic events, 5 files of 1 MiB at most by default, deleted after 14 days. |

[Privacy](privacy.md) lists what each file holds and every network request.

## Updates

- The build installed by `Setup.exe` (Velopack) checks
  `https://worktime.thanaen.dev/updates/` at most once a day, downloads an
  update in the background, and installs it at the next start or with
  « Redémarrer WorkTime ».
- The Microsoft Store build asks the Store, at most once a day.
- The portable ZIP never checks; its settings link to the GitHub releases page.
- « Rechercher les mises à jour » in Settings turns the checks off.
- Only the `Setup.exe` build carries Velopack: it is built with worktime-app's
  `direct` feature, the Store package and the ZIP with its `release` feature.

## Testability

The rules never reference GPUI or Win32 types, so the domain, the store, the
services and the view models are tested on Linux. `worktime-app` draws every
demo scenario off screen with `worktime-screenshots`, which CI uses to publish
screenshots of each pull request. See [CONTRIBUTING.md](../CONTRIBUTING.md).

## Related documents

- [Product specification](../specs/001-core-workday-indicator/spec.md)
- [Data model](../specs/001-core-workday-indicator/data-model.md)
- [Built-in activity source contract](../specs/001-core-workday-indicator/contracts/built-in-activity-source.md)
- [Windows power history recovery](power-history-recovery.md)
- [Start with Windows](startup-registration.md)
- [Product decisions](product-decisions.md)
- [Privacy](privacy.md)
- [CI quality gates](ci-quality-gates.md)
