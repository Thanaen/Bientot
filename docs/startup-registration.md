# Start with Windows

Open the Bientôt settings and turn on **Start with Windows**. The toggle
loads the actual Windows state. An old `StartWithWindows = true` setting does
not register the application automatically. Saving another setting also reads
the current Windows state without changing the startup registration.

## Installer and portable application

The build installed by `Setup.exe` and the portable ZIP register the same way.
For the ZIP, extract the archive to the folder you intend to keep before enabling startup.
Bientôt registers the quoted full executable path in the current user's
`Software\Microsoft\Windows\CurrentVersion\Run` key with the private launch
marker `--worktime-windows-startup`. For example:

```text
"C:\Apps\Bientot\worktime.exe" --worktime-windows-startup
```

The marker tells Bientôt to start accounting, notifications, and the tray
without opening the flyout. A manual launch still opens it. If hidden
initialization fails, Bientôt opens the startup failure window.

The Run value is named `WorkTime`, and the marker keeps its `worktime`
prefix. Both date from before the app was renamed Bientôt, and copies
installed then still depend on them, so they do not change.

Portable registration does not require an administrator or create a scheduled
task.

If you move the folder, open Bientôt from its new location and explicitly
enable startup again. Settings report the old registration as a mismatch.
Disabling startup removes only a registration pointing to the current
executable. A development host or missing executable cannot be registered.

If Windows Task Manager has disabled Bientôt, settings explain that state.
Re-enable it through Windows. Bientôt reads the Windows startup approval
state and never changes or deletes that approval.

When Bientôt finds the previous path-only command for its current executable,
it updates that exact command to include the marker. It updates only entries
that Windows reports as enabled or without an approval record. It leaves
disabled and unknown approval states unchanged. A command for another
executable remains a mismatch. The migration never writes or deletes the
`StartupApproved\Run` value.

The old path-only command cannot distinguish a sign-in launch from a manual
launch. If that command starts the updated app before migration runs, Bientôt
opens the window once and updates the owned command for the next sign-in.

## Packaged application

The MSIX declares `WorkTimeStartupTask` with `Enabled="false"`. Explicit enable
uses the Windows `StartupTask` API. Settings distinguish user disablement,
policy disablement, and policy enablement. Bientôt does not override those
states. At launch, Bientôt hides the initial window only when Windows reports
the exact `WorkTimeStartupTask` activation.

Demo mode disables startup registration so an isolated demo cannot register
its test executable as the real Bientôt startup entry.

## Moving to the Store build

The Store build manages only its own startup task. It does not remove the Run
value of an installer or portable copy, which starts another executable. After
moving to the Store build, uninstall the installer copy, which removes its
Run value. Otherwise both start at sign-in. Only one runs, because a second
copy hands over to the one that already holds the data folder, but it may be
the older copy. For a portable copy, quit the Store build first, start the
portable copy, and turn off **Windows startup** in its settings: while the
Store build runs, starting the portable copy only brings up the Store build,
whose toggle controls its own startup task. #124 tracks a fix in the app.

## Verification

The `bientot-windows` tests cover the registry states, path quoting,
registration mismatches and Windows overrides; the `bientot-app` tests cover
the settings rows. The registry tests run only on Windows (`cargo test -p
bientot-windows`).

To observe window visibility without changing startup registration, build
`worktime.exe` with `cargo build --release -p bientot-app --bin worktime` and run:

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

The Store install, the startup task, and the uninstall were checked for v1.0.0;
see [release-checklist.md](release-checklist.md#microsoft-store-package). A
real sign-in launch and a Store update remain manual checks.
