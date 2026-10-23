# Quickstart and Validation Scenarios

**Status**: Design-time guide; commands become executable after project setup.

## Development prerequisites

- Windows 11 x64
- Visual Studio with WinUI / Windows App SDK workload
- .NET 10 SDK
- PowerShell 7
- optional: local ActivityWatch using `aw-server-rust` or Python `aw-server`,
  to validate the ActivityWatch source (the built-in source needs nothing)

## Expected first implementation commands

```powershell
dotnet restore WorkTime.slnx
dotnet build WorkTime.slnx -c Debug
dotnet test WorkTime.slnx -c Debug
dotnet run --project src/WorkTime.App/WorkTime.App.csproj
```

Exact solution and package versions are created by the setup tasks.

## Source validation

The built-in source needs no preparation. To validate the ActivityWatch
source, check the server before running WorkTime:

```powershell
Invoke-RestMethod http://127.0.0.1:5600/api/0/info
Invoke-RestMethod http://127.0.0.1:5600/api/0/buckets/
```

If bearer authentication is enabled, provide the key interactively; do not
place it in repository files or shell history.

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
6. Before 14:00, no numeric departure is shown.
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

1. With ActivityWatch selected, stop ActivityWatch; verify stale/degraded
   state, no crash, no false zero.
2. Restart it; verify automatic recovery.
3. Suspend while locked; verify sleeping time is excluded.
4. Start WorkTime after morning activity; verify known activity is credited and
   unknown session gaps remain visible. With the built-in source, the time
   before launch is unknown; with ActivityWatch, its recorded activity is
   credited.
5. Restart after notifications fired; verify no duplicate.
6. With the built-in source, leave the keyboard and mouse untouched for longer
   than the idle threshold; verify the idle time is excluded from the last
   input, not from the moment it was detected.
7. With the built-in source, end the WorkTime process from Task Manager and
   start it again; verify the interval in between is unknown and not credited.
   Repeat with a clean exit from the tray less than the idle threshold after
   the last input; verify the time after the last input is unknown, also when
   read again hours later.
8. Switch the source in the middle of a workday; verify each part of the day
   is read from the source selected at that time, never from both.

## Release smoke scenario

On a clean Windows 11 x64 VM:

1. Install WorkTime from the Microsoft Store and complete French onboarding
   without ActivityWatch installed; verify no ActivityWatch step appears and
   the built-in source is healthy. Repeat with ActivityWatch running and verify
   onboarding offers it. For an alpha, extract the portable ZIP from
   `alpha-latest` instead. After onboarding with the built-in source, verify
   that the time before confirmation is unknown, even with ActivityWatch
   running. Upgrade a build from before D-021 configured with ActivityWatch
   and verify earlier days still read ActivityWatch.
2. Verify start-with-Windows consent and sign-in launch.
3. Verify tray recovery after Explorer restart.
4. Verify English override and resource completeness.
5. Verify portable alpha limitations are documented.
6. Verify update offer requires user action.
7. Export diagnostics and confirm no secret, app title, or window title.
8. Measure usable tray startup under 3 seconds, average idle CPU under 0.5%,
   and working set at or below 150 MiB.

## Start-with-Windows visibility scenario

Use an isolated portable build before the clean-VM release smoke:

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

The script observes one launched process for eight seconds and writes
`visibility.json`. It does not simulate a Windows sign-in. Validate the signed
MSIX and `WorkTimeStartupTask` payload on a clean Windows 11 x64 VM.

### Diagnostic export smoke

1. Start WorkTime and immediately choose **Export diagnostics…** in the main
   window, including while initialization is still running or after a controlled
   startup failure.
2. Repeat from **Export diagnostics…** in the tray menu. Verify that WorkTime is
   brought into view and the Windows save picker suggests
   `worktime-diagnostics.zip` with the ZIP file type.
3. Cancel the picker. Verify that WorkTime adds no success or failure notice and does not
   create or replace a file.
4. Complete the export and confirm the archive contains only `manifest.json`,
   `snapshot.json`, and `logs.jsonl`. Verify the success notice and confirm that
   the ZIP is saved only to the destination chosen in the picker, with no
   automatic upload.
5. Inspect every entry for test API keys, bucket and machine identifiers, local
   paths, ActivityWatch application names, and window titles. None may appear;
   logs contain only the documented finite events and values.
6. In a controlled test location where publication is denied, verify the
   localized failure notice contains no exception or path and an existing
   destination remains unchanged.

The symbolic-link boundary test needs Developer Mode or the Windows permission
to create symbolic links. Its normal skip is explicit. To run it on a prepared
host, set `WORKTIME_NATIVE_DIAGNOSTIC_LINK_TEST=1` and run the
`DiagnosticPrivacyTests.ExportAsync_RejectsReparseRoot` test. A missing permission
then fails the test rather than silently passing it.

### T057 lifecycle smoke boundary

The automated T057 tests exercise startup probes, deterministic callback keys,
stream consumption, SQLite persistence, restart replay, and refresh requests.
They do not prove that Windows delivers real `WM_WTSSESSION_CHANGE` and
`WM_POWERBROADCAST` messages to the WinUI window handle. Before calling the
Windows lifecycle path release-ready, run the packaged app on Windows 11 x64,
lock and unlock the active session, then suspend and resume the machine. Verify
that each transition appears once in `SessionEvent` and changes the published
summary. This remains a manual smoke gap until the clean-VM lifecycle harness
exists.

The integration test retries adjustments with an explicit stable ID. The UI
creates one ID before submission and keeps it while the open dialog retries;
there is no automatic cross-restart retry of an interrupted dialog submission.
Editing appends a superseding adjustment; removal remains unavailable. Do not
treat UI restart retry or adjustment removal as covered by T057.
