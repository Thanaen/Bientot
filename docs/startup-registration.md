# Start at sign-in

Open the Bientôt settings and turn on **Windows startup** on Windows, or
**Automatic startup** on Linux. The toggle loads the actual state of the
system. An old `StartWithWindows = true` setting does not register the
application automatically. Saving another setting also reads the current
state without changing the startup registration.

The sections below up to [Linux](#linux) describe Windows.

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
Bientôt also reads `--autostart`, the marker its Linux autostart entry
writes, with the same meaning. Windows registrations keep writing their own.

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
states. When you turned Bientôt off in Startup apps, **Windows startup** says so
and offers an **Open Startup apps** button, since only that page can turn
it back on. At launch, Bientôt hides the initial window only when Windows reports
the exact `WorkTimeStartupTask` activation.

Demo mode disables startup registration so an isolated demo cannot register
its test executable as the real Bientôt startup entry.

## Moving to the Store build

An installer or portable copy keeps its `WorkTime` Run value when you move to
the Store build. Both would then start at sign-in. Only one runs, because a
second copy hands over to the one that already holds the data folder, but it
may be the older copy.

When the Store build's startup task is on (enabled, or enabled by policy), a
`WorkTime` Run value exists, and Windows has not recorded it as disabled in
`StartupApproved\Run`, **Windows startup** in the Store build's settings says
that another copy of Bientôt also starts with Windows. Its **Open Startup
apps** button opens Windows Settings > Apps > Startup
(`ms-settings:startupapps`), where you turn the other copy off. The line is read
when the settings window opens, after the toggle changes and whenever the window
becomes active again, so it goes away once you come back from Startup apps.

The Store build cannot remove that Run value itself. Windows keeps the
packaged app's writes under `HKEY_CURRENT_USER` in a private per-app copy, and
reads merge that copy with the real registry. A delete would only hide the
value from the Store build, and the other copy would still start. The Store
build therefore only reads the Run value and its approval, and never writes
either. The installer and portable copies never show this line, and they
manage their own Run value as described above.

Uninstalling the installer copy also removes its Run value.

## Linux

The AppImage and a build from source register with an XDG autostart entry,
`$XDG_CONFIG_HOME/autostart/dev.bientot.Bientot.desktop`, or
`~/.config/autostart/dev.bientot.Bientot.desktop` when `XDG_CONFIG_HOME` is
unset or not an absolute path. The desktop starts every entry of that folder
when you sign in. Its `Exec` line names the AppImage file itself, since the
executable inside it moves at every run, or the executable of a build from
source, with the `--autostart` marker. For example:

```ini
[Desktop Entry]
Type=Application
Name=Bientôt
Exec="/home/user/Applications/Bientot.AppImage" --autostart
Icon=dev.bientot.Bientot
X-GNOME-Autostart-enabled=true
```

`--autostart` has the meaning of `--worktime-windows-startup` on Windows: a
sign-in launch shows only the tray icon. Without a tray host it shows its
window.

The setting reads the entry. It is off when the file is missing, or when it
has `Hidden=true` or `X-GNOME-Autostart-enabled=false`, since the desktop then
skips it. Settings report an entry that starts another file as a mismatch,
for example after you moved the AppImage. Turn the setting on again from the
new file to replace it. Turning the setting on rewrites the entry unless it
already starts this copy. Turning it off deletes the file only when it starts
this copy.

A Flatpak's sandbox has an autostart folder of its own, which no desktop
reads, so a Flatpak shows startup registration as unavailable. A demo
registers nothing, as on Windows.

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

On Linux, the `bientot-linux` tests write, read and remove the autostart
entry in a temporary configuration folder, on any OS
(`cargo test -p bientot-linux`).
`crates/bientot-app/tests/desktop_harness_instance.rs` starts the app with
`--autostart` in the desktop harness and checks that it shows nothing
([CONTRIBUTING.md](../CONTRIBUTING.md#check-both-platforms)).

The Store install, the startup task, and the uninstall were checked for v1.0.0;
see [release-checklist.md](release-checklist.md#microsoft-store-package). A
real sign-in launch and a Store update remain manual checks.
