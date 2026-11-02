# Quickstart and Validation Scenarios

**Status**: Executable against the Rust app. The `docs/validation-report.md`
records each run of these scenarios, `docs/release-checklist.md` the release
smoke and Store package checks, and `docs/accessibility-checklist.md` the
accessibility checks.

The first version of the app, then called WorkTime, was a .NET app that could
also read ActivityWatch. It was removed in October 2026 (D-022). The steps
that only applied to ActivityWatch now check what an upgraded user sees.

## Development prerequisites

- Windows 11 x64 to run the app; the tests also run on Linux and macOS
- `rustup`; the toolchain of `rust-toolchain.toml` installs on the first
  `cargo` command
- PowerShell 7 (`pwsh`) for the scripts in `scripts/`

## Commands

```powershell
cargo test --workspace --all-features --locked
cargo build --release --locked -p bientot-app --bin worktime --features release
target\release\worktime.exe --demo
```

`--demo [scenario]` opens a throwaway data folder with a fixed clock. The
scenarios are listed in `crates/bientot-app/src/demo.rs`: `on-track`,
`near-limit`, `late`, `goal-reached`, `morning`, `morning-near-limit`,
`morning-late`, `credit`, `history-week`, `problem-periods`,
`fragmented-day`, `source-unavailable` and `source-activitywatch`.
`--data-dir <folder>` runs on a folder of your choice; `--english`,
`--french`, `--light`, `--dark`, `--onboarding` and `--demo-update` override
what the app would pick.

## Core scenario

Configuration:

- target 7h;
- lunch 12:00–14:00;
- latest departure 18:30;
- orange margin 15 min;
- minimum effective target 6h.

Validate:

1. Active morning time is credited.
2. A lock from 12:15–13:00 is excluded as lunch.
3. Active time during lunch remains credited.
4. A lock outside lunch is provisional work and is flagged after 90 minutes.
5. A lock left active past the end of the workday is excluded as
   `overnight-lock` on both days until the unlock, with no review.
6. Before 14:00, the departure time is shown with the line that lunch is
   planned until 14:00 (D-015, revised on 2026-10-23).
7. After 14:00, green/orange/red matches exact threshold tests.
8. At target completion, state becomes blue and overtime grows only with newly
   credited time.

## Schedule settings scroll and keyboard scenario

1. Open Schedule settings in a window short enough for the dialog to scroll.
   Enable both approach notification toggles so their lead-time fields are editable.
2. Scroll through all eight numeric settings; verify each field keeps its step
   buttons beside the value and inside the scrolling content.
3. Use Tab and the arrow keys to change each numeric setting, save, and reopen
   the dialog; verify every value persists.

## Weekly credit scenario

1. Work 7h30 Monday; verify +30 min available.
2. Work 6h30 Tuesday; verify blue with 30 min applied and zero new overtime.
3. Correct Monday down by 10 min; verify Tuesday and balance recompute.
4. Create more credit than the 6h floor permits; verify the remainder stays
   available through Friday.
5. Start the next Monday; verify Friday remainder expired.

## Resilience scenario

1. When the built-in source cannot read input, verify the degraded state, no
   crash and no false zero. (Was: ActivityWatch stopped.)
2. When it reads input again, verify automatic recovery. (Was: ActivityWatch
   restarted.)
3. Suspend while locked; verify sleeping time is excluded.
4. Start Bientôt after morning activity; verify the session before the first
   start is rebuilt from the Windows event logs, counts provisionally and is
   offered for review, and that any other time before launch stays unknown.
5. Restart after notifications fired; verify no duplicate.
6. Leave the keyboard and mouse untouched for longer than the idle threshold;
   verify the idle time is excluded from the last input, not from the moment
   it was detected.
7. End the Bientôt process from Task Manager and start it again; verify the
   interval in between is unknown and not credited. Repeat with a clean exit
   from the tray less than the idle threshold after the last input; verify the
   time after the last input is unknown, also when read again hours later.
8. Only a one-time switch from ActivityWatch to the built-in source remains,
   for a user upgraded from the first version. Verify each part of that day is
   read from the source selected at that time, never from both, and that time
   selected for ActivityWatch reads as unavailable.

## Release smoke scenario

On a clean Windows 11 x64 machine:

1. Install Bientôt from the Microsoft Store and complete French onboarding;
   verify no ActivityWatch step appears and the built-in source is healthy.
   For an alpha, install `Bientot-Setup.exe` or extract `Bientot-Portable.zip`
   from the latest GitHub release instead. After onboarding, verify that the
   time before confirmation is unknown. Upgrade a data folder from the first
   version configured with ActivityWatch and verify earlier days keep their
   stored totals.
2. Verify start-with-Windows consent and sign-in launch.
3. Verify tray recovery after Explorer restart.
4. Verify English override and resource completeness.
5. Verify portable alpha limitations are documented.
6. Verify update offer requires user action.
7. Export diagnostics and confirm no secret, app title, or window title.
8. Measure usable tray startup under 3 seconds, average idle CPU under 0.5%,
   and working set at or below 150 MiB with
   `scripts/measure-performance.ps1 -Release`.

## Start-with-Windows visibility scenario

Use an isolated portable build before the clean-machine release smoke:

1. Run `scripts/verify-startup-visibility.ps1` in `Startup` mode. Verify that
   the process stays alive, the demo database appears, and no top-level window
   show event or visible main window appears.
2. Run it in `Manual` mode. Verify that the main window becomes visible.
3. Run it in `Failure` mode. Verify that the hidden startup opens the main
   window. Inspect the window separately to confirm that it shows the
   initialization error.
4. Start the demo with the startup marker. Verify that it does not create or
   change the real `WorkTime` startup registration.
5. Close a normal initialized window to the tray. Verify that a tray activation
   and a notification activation each reopen the existing window.
   `scripts/test-rust-toast-activation.ps1` checks a notification activation
   of a stopped app.

The script observes one launched process for eight seconds and writes
`visibility.json`. It does not simulate a Windows sign-in. Validate the
Store-signed MSIX and its `WorkTimeStartupTask` on a clean Windows 11 x64
machine.

### Diagnostic export smoke

1. Start Bientôt and immediately choose **Export diagnostics…** in the main
   window, including while initialization is still running or after a controlled
   startup failure.
2. Repeat from **Export diagnostics…** in the tray menu. Verify that Bientôt is
   brought into view and the Windows save picker suggests
   `bientot-diagnostics.zip` with the ZIP file type.
3. Cancel the picker. Verify that Bientôt adds no success or failure notice and
   does not create or replace a file.
4. Complete the export and confirm the archive contains only `manifest.json`,
   `snapshot.json`, and `logs.jsonl`. Verify the success notice and confirm that
   the ZIP is saved only to the destination chosen in the picker, with no
   automatic upload.
5. Inspect every entry for test API keys, bucket and machine identifiers, local
   paths, application names, and window titles. None may appear; logs contain
   only the documented finite events and values.
6. In a controlled test location where publication is denied, verify the
   localized failure notice contains no exception or path and an existing
   destination remains unchanged.

The symbolic-link boundary test (`rejects_reparse_root` in
`crates/bientot-core/tests/diagnostics_export.rs`) runs on Unix only. On
Windows, `is_reparse_point` in `crates/bientot-core/src/diagnostics/export.rs`
checks the reparse-point attribute; check a real junction by hand.

### Lifecycle smoke boundary

The automated lifecycle tests exercise startup probes, deterministic callback
keys, SQLite persistence and restart replay. The Windows-only
`messages_of_this_session_reach_the_host_until_the_window_is_dropped` test
posts `WM_WTSSESSION_CHANGE` and `WM_POWERBROADCAST` to the app's message
window, which proves the decoding, not that Windows delivers them. Before
calling the lifecycle path release-ready, run the packaged app on Windows 11
x64, lock and unlock the active session, then suspend and resume the machine.
Verify that each transition is recorded once and changes the published
summary.

The adjustment dialog creates one ID before submission and keeps it while it
retries; there is no automatic cross-restart retry of an interrupted dialog
submission. Editing appends a superseding adjustment, and a correction can be
revoked.
