# Troubleshooting

Each section starts with the text Bientôt shows. Messages come from
`crates/bientot-app/strings/en-US.resw`; the French build shows the
same messages from `fr-FR`.

## Activity source problems

Bientôt detects activity on its own with its built-in source. When the
source fails, Bientôt keeps the last good values. The tray icon gets a
degraded badge, its tooltip reads "*reason* · Open Bientôt", and the flyout
explains the problem. F5 in the flyout tries again at once. When the source
works but left time unrecorded, the icon keeps the badge and its tooltip ends
with the reason, for example "Unrecorded time".

Time before onboarding was confirmed has no source. For it the tooltip reads
"No activity source" and the banner "No activity source is selected for this
period. That time stays unrecorded." Add a manual correction if you worked
then.

### Built-in source

The built-in source checks the time of your last keyboard or mouse input every
5 seconds while Bientôt runs. After the idle threshold without input (180 s
by default), you count as away from your last input.

| Tray tooltip | Window message | What to do |
| --- | --- | --- |
| Cannot read input | Bientôt cannot read when you last used the keyboard or mouse. That time stays unrecorded. Reading resumes on its own. | Windows refused the last-input query (`GetLastInputInfo`). Wait for the next sample. If the message stays, restart Bientôt and export diagnostics. |
| Activity recording stopped | Activity recording has stopped. It resumes on its own. If this message stays, restart Bientôt. | No recent sample was recorded, for example while the process was stalled. It resumes at the next sample. |

Time when Bientôt was not running is unknown, and so is the time after your
last input when Bientôt stops before the idle threshold has passed. Bientôt
never credits that time automatically. Start Bientôt with Windows to avoid
gaps, and add a manual correction for time you worked while it was closed.

To count short breaks as work, raise the idle threshold in the settings
(30 seconds to 30 minutes). The new threshold applies from the change.
Stored periods are not recalculated.

### Time selected for ActivityWatch

The first version of the app could also read ActivityWatch. The current
version reads only its built-in source. Days already recorded from
ActivityWatch keep their stored totals; time that was selected for
ActivityWatch but never recorded shows as unknown. Add a manual correction if
you worked then.

## Start with Windows

The settings show the real Windows state, not only the saved choice.

| Message | Meaning |
| --- | --- |
| Windows Startup Apps has disabled Bientôt. Re-enable it in Settings > Apps > Startup. | You or Task Manager disabled it. Bientôt never overrides this. |
| Your organization prevents Bientôt from starting when you sign in. | A policy blocks it. |
| Your organization starts Bientôt when you sign in. | A policy enables it. |
| Another copy uses the Bientôt startup entry. Turn this setting on to use this copy. | The portable folder moved, or another copy registered itself. |
| Startup registration is unavailable for this copy of Bientôt. | Demo mode, a development build, or a missing executable. |
| Windows did not apply the startup change. Check the status above and try again. | Windows rejected the change. |

A sign-in launch opens no window, only the tray icon. See
[Start with Windows](startup-registration.md) for the portable and packaged
mechanisms.

## The tray icon does not appear

- Bientôt adds the icon after it has loaded the settings and the database and
  run the first refresh. A sign-in launch shows nothing until then.
- If startup fails, Bientôt opens a window with "Bientôt could not start.
  Check the local settings and database, then restart the app." Export
  diagnostics from that window.
- Windows 11 can hide notification-area icons behind the overflow arrow. Turn
  Bientôt on under **Settings > Personalization > Taskbar > Other system tray
  icons**. This is a Windows setting, not a Bientôt one.
- Closing the flyout or a window keeps Bientôt running in the tray. Use **Quit
  Bientôt** in the tray menu to stop it.
- Only one copy of Bientôt runs per data directory. Starting it again, for example
  from the Start menu after a sign-in launch, opens the running copy's flyout
  instead of adding a second tray icon. A run with `--data-dir` or a demo
  flag uses its own folder and starts separately.

## Other messages

| Message | Meaning |
| --- | --- |
| Invalid settings were backed up and the defaults restored. Review your settings. | `settings.json` could not be read. The old file is kept as `settings.json.invalid-<guid>.json` in `%LOCALAPPDATA%\Bientot`. |
| Settings could not be saved. Check disk access and try again. | Writing `settings.json` failed. |
| Windows notifications are unavailable. Workday tracking continues. | Notification registration failed at startup. |
| Bientôt could not show a notification. Workday tracking continues. | Windows refused one notification. |
| Bientôt could not refresh. It will try again automatically. | A refresh failed. The next one runs within 30 s. |
| The command failed. Try again. Your current tracking state was not changed. | A tray command such as Pause or Finish failed. |

## Updates

- The build installed by `Setup.exe` downloads a new version once a day and
  shows "A new version is ready. It will install the next time Bientôt
  starts." Select **Restart Bientôt** to install it now.
- The Microsoft Store build shows "A new version is available in the
  Microsoft Store. Bientôt may close while it installs." Select **Update** to
  hand the update to the Store.
- The portable ZIP never checks. Download a newer version from
  [bientot.dev/download](https://bientot.dev/download) or
  the GitHub releases page.
- No offer appears: Bientôt already checked today, the check failed, or
  **Check for updates** is off in the General settings. Failed checks are
  logged as `update_check.failed`.

## Collect diagnostics

1. Select **Export diagnostics…** in the tray menu.
2. Choose where to save `bientot-diagnostics.zip`.
3. Look for "The diagnostic ZIP was saved." If you see "Bientôt could not
   save the diagnostic ZIP. Choose another location and try again.", pick a
   folder you can write to.
4. Attach the ZIP to a
   [new issue](https://github.com/Thanaen/Bientot/issues/new/choose).

The ZIP contains numeric settings, today's totals and source health, and
filtered log events. It does not contain paths, error messages, application names or window titles. See
[Privacy](privacy.md#diagnostic-export).

The raw logs are in `%LOCALAPPDATA%\Bientot\logs`. They are redacted when
written but hold more detail than the export, such as exception types and
redacted error text. Read them before sharing them.
