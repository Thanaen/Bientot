# Privacy

Bientôt is local-first. It has no account and no cloud sync. Its only
telemetry is one anonymous usage signal a day. It is on by default, shown
during setup, and you can turn it off. This page lists what Bientôt stores,
what it sends, and how to remove it.

## What stays on the machine

Spec SC-005: no activity or accounting data leaves the machine. Bientôt never
uploads its accounting records. Its only remote requests are the daily update
check and the anonymous daily usage signal described below.

The built-in activity source reads one value from Windows: the time of your
last keyboard or mouse input (`GetLastInputInfo`). It never reads what you
type, which keys you press, application names or window titles. It stores only
the start and settled end of each active or idle period.

Bientôt stores everything in `%LOCALAPPDATA%\Bientot`. Builds released
before the rename to Bientôt used `%LOCALAPPDATA%\WorkTime`, and the first
start of a newer build renames that folder to `Bientot`. A run started with
`--data-dir <folder>` uses that folder instead, and a `--demo` run uses a
temporary `BientotDemo` folder.

| File | Content |
| --- | --- |
| `settings.json` | Schedule, recurring appointments, notification rules, language, the built-in idle threshold, the time of the last update check, whether the anonymous usage signal is on. It also keeps the ActivityWatch address and timings of the first version of the app, which the current version no longer uses. |
| `settings.json.invalid-<id>.json` | A copy of a settings file Bientôt could not read. It is kept so nothing is lost when defaults are restored. |
| `app-state.json` | The date of the last usage signal the server recorded, and which appointment notices you closed in the flyout today (internal ids of the correction and the appointment, with the date). Nothing else. |
| `worktime.db` | SQLite database: which activity source was selected from which time, the active and idle periods, session, lock and power events with their times, pause and finish commands, manual corrections with their optional reason, long-lock decisions, daily summaries, weekly credit, and which notifications were already shown. |
| `secrets\*.secret` | Secrets encrypted with DPAPI for your Windows account. Only the ActivityWatch API key of the first version of the app was ever stored there; the current version writes none. |
| `import.worktime` | An import chosen in **Settings › Data** or on the first step of the onboarding, waiting for the next start to apply it. |
| `backups\` | The data an import replaced (**Settings › Data**), saved as a `.bientot` file (`.worktime` before the rename) so that its **Restore…** button can undo the import. |
| `logs\worktime-*.jsonl` | Diagnostic events, at most 5 files of 1 MiB, each deleted after 14 days. |

The diagnostic log also redacts at write time. Properties named like an API
key, token, password, secret, window title, application name, message or
payload are replaced with `[REDACTED]`. Bearer tokens and `key=value` secrets
inside strings are replaced too.

Accounting history does not expire automatically.

A `.bientot` file exported from **Settings › Data**, or a `.worktime` file
exported before the rename, holds a copy of the database and of
`settings.json`. It stays where you save it; Bientôt never sends it
anywhere.

## Network requests

Bientôt makes two kinds of request.

1. **Update check.** At most once a day, and only while **Check for updates**
   is on in the General settings.
   - The build installed by `Setup.exe` sends one `GET` request to
     `https://worktime.thanaen.dev/updates/releases.win.json`, with no query
     string, no cookie and no identifier, and downloads a newer package from
     the same address. Both answer with a redirect to the same file in
     the GitHub Releases of `Thanaen/WorkTime`, which GitHub serves from
     `github.com` and a `*.githubusercontent.com` address. Those receive the
     same request.
   - The Microsoft Store build asks the Store through the Windows
     `StoreContext` API.
   - The portable ZIP never checks.

   Turning the toggle off saves `"checkForUpdates": false` under `"updates"`
   in `settings.json`.
2. **Anonymous usage signal.** At most once a day, release builds send one
   `POST` request to `https://worktime.thanaen.dev/v1/ping` with this body and
   nothing else:

   ```json
   {"version":"1.0.0","channel":"store","first_this_month":true}
   ```

   `version` is the Bientôt version. `channel` is `store`, `direct` or `zip`,
   depending on where you installed Bientôt. `first_this_month` is `true` for
   the first signal of a calendar month. The request carries no identifier, no
   cookie, no date and no time-tracking data. The server stores nothing and logs
   nothing (`services/worktime`). It passes the three fields on to
   [PostHog](https://posthog.com/privacy), hosted in the European Union
   (Frankfurt), as one event dated at midnight UTC of the day, with a random id
   used only once, no IP address and no person profile. PostHog keeps each
   signal as an anonymous event, and never sees the address of your PC: the
   request to PostHog comes from the server. Bientôt keeps only
   the date of the last signal the server recorded, in `app-state.json`. After
   a failure, Bientôt tries again at most once an hour. Debug builds, demo
   launches and launches with `--data-dir` never send it. Turn off **Send an
   anonymous usage signal** in the General settings to stop it at once,
   without a restart. The toggle saves `"usagePing": false` in
   `settings.json`.

   These counts are public, at `https://worktime.thanaen.dev/statistiques`, as a
   page and as CSV and JSON files under CC0. The page shows only finished days
   and finished months, no figure under 10 (small groups are merged until they
   reach 10), and the figures per version and channel rounded to the nearest 5,
   so that no figure points at a few installations.

## The website

The app never loads the website. The pages of `https://worktime.thanaen.dev`
measure their audience and loading speed with PostHog, hosted in the European
Union, through `https://e.worktime.thanaen.dev`, an address PostHog runs for
the site. The script sets no cookie and stores nothing in the browser
(cookieless mode). PostHog counts a visit with a hash of the IP address and
the browser that changes every day, and keeps no IP address. For each page
view, it keeps the time, the page address, the referring site, the operating
system, the browser, the device type, the screen size, the language and the
time zone, and the loading-speed measurements of the page (Core Web Vitals). It records no
clicks, no keystrokes and no session replay. PostHog's
[privacy page](https://posthog.com/privacy) gives the details. A content
blocker that stops `e.worktime.thanaen.dev` stops it.

## Diagnostic export

**Export diagnostics…** in the tray menu saves a ZIP where you choose.
Bientôt never sends it anywhere. The ZIP holds three files:

- `manifest.json`: the export limits, what was available, and counts of the
  log lines that were left out and why.
- `snapshot.json`: numeric settings (targets, lunch window, departure time,
  margins, notification rules, log limits, start with Windows, update check),
  the version numbers of Bientôt and Windows, the processor architecture, and
  today's totals, indicator colour and source health.
- `logs.jsonl`: up to 10,000 log records from the newest 5 log files of the
  last 14 days. Each record keeps only its time (rounded to the minute), its
  level, an event name from a fixed list, a failure category such as `Io`, the
  status of an unavailable power history, and for a failed Store update check
  its numeric error code (HRESULT).

The export leaves out file paths, host names, error messages, application
names, window titles and any log event outside the fixed list. The tests in
`crates/worktime-core/tests/diagnostics_export.rs` plant such values in the
logs and settings and check that none of them reaches the archive.

Cancelling the save dialog writes nothing. An existing file is replaced only
after the new archive is complete.

## Delete your data

1. If **Start with Windows** is on, turn it off in the Bientôt
   settings. Otherwise Bientôt starts again at the next sign-in.
2. Quit Bientôt from the tray menu.
3. Delete the `%LOCALAPPDATA%\Bientot` folder, and `%LOCALAPPDATA%\WorkTime`
   if it is still there, or the folder you passed with `--data-dir` if you
   start Bientôt that way.
4. Delete any `.bientot` or `.worktime` file you exported.

The next start from the same data folder shows onboarding again and creates a
new, empty database.

The Store build can also keep data in its own package folder, because Windows
virtualizes packaged writes under `%LOCALAPPDATA%`. Uninstalling the app
removes that package data.
