# Start with Windows

Open WorkTime settings and turn on **Start with Windows**. The toggle
loads the actual Windows state. An old `StartWithWindows = true` setting does
not register the application automatically. Saving another setting also reads
the current Windows state without changing the startup registration.

## Installer and portable application

The build installed by `Setup.exe` and the portable ZIP register the same way.
For the ZIP, extract the archive to the folder you intend to keep before enabling startup.
WorkTime registers the quoted full executable path in the current user's
`Software\Microsoft\Windows\CurrentVersion\Run` key with the private launch
marker `--worktime-windows-startup`. For example:

```text
"C:\Apps\WorkTime\worktime.exe" --worktime-windows-startup
```

The marker tells WorkTime to start accounting, notifications, and the tray
without opening the flyout. A manual launch still opens it. If hidden
initialization fails, WorkTime opens the startup failure window.

Portable registration does not require an administrator or create a scheduled
task.

If you move the folder, open WorkTime from its new location and explicitly
enable startup again. Settings report the old registration as a mismatch.
Disabling startup removes only a registration pointing to the current
executable. A development host or missing executable cannot be registered.

If Windows Task Manager has disabled WorkTime, settings explain that state.
Re-enable it through Windows. WorkTime reads the Windows startup approval
state and never changes or deletes that approval.

When WorkTime finds the previous path-only command for its current executable,
it updates that exact command to include the marker. It updates only entries
that Windows reports as enabled or without an approval record. It leaves
disabled and unknown approval states unchanged. A command for another
executable remains a mismatch. The migration never writes or deletes the
`StartupApproved\Run` value.

The old path-only command cannot distinguish a sign-in launch from a manual
launch. If that command starts the updated app before migration runs, WorkTime
opens the window once and updates the owned command for the next sign-in.

## Packaged application

The MSIX declares `WorkTimeStartupTask` with `Enabled="false"`. Explicit enable
uses the Windows `StartupTask` API. Settings distinguish user disablement,
policy disablement, and policy enablement. WorkTime does not override those
states. At launch, WorkTime hides the initial window only when Windows reports
the exact `WorkTimeStartupTask` activation.

Demo mode disables startup registration so an isolated demo cannot register
its test executable as the real WorkTime startup entry.

## Verification

The `worktime-windows` tests cover the registry states, path quoting,
registration mismatches and Windows overrides; the `worktime-app` tests cover
the settings rows. The registry tests run only on Windows (`cargo test -p
worktime-windows`).

To observe window visibility without changing startup registration, build
`worktime.exe` with `cargo build --release -p worktime-app --bin worktime` and run:

```powershell
.\scripts\verify-startup-visibility.ps1 `
    -ExecutablePath target\release\worktime.exe `
    -Mode Startup `
    -OutputDirectory <fresh-output-directory>
```

Run the same command with `-Mode Manual` and `-Mode Failure`. The verifier uses
isolated demo data. It records whether the process stays alive, whether the demo
database appears, top-level window show events, visible windows, and the startup
failure window. It watches for eight seconds, writes `visibility.json`, and stops
that process during cleanup. It does not simulate a Windows sign-in or validate
a Store-signed MSIX.

Installation from the Microsoft Store, Windows
sign-in, packaged startup-task payload, upgrade, and uninstall validation remain
release acceptance checks.
