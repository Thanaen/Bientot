# Validation report

This report records each scenario of
[`quickstart.md`](../specs/001-core-workday-indicator/quickstart.md) against
the Rust app before v1.0.0 (T116), and [the plugin runtime](#plugin-runtime)
before it ships (T065). The release smoke and Store package checks
are in [release-checklist.md](release-checklist.md), and the accessibility
checks in [accessibility-checklist.md](accessibility-checklist.md).

## Run

- **Date.** 2 November 2026, 19:56 to 21:00 (Paris time).
- **Machine.** The maintainer's PC: Windows 11 Pro 10.0.26300 x64, AMD
  Ryzen 7 9800X3D (16 logical processors), 2560 × 1440 at 125 %, French
  locale. Bientôt had never been installed on it: no package, no
  `%LOCALAPPDATA%\Bientot`. It is not a clean virtual machine; see
  [release-checklist.md](release-checklist.md#machine).
- **Builds.**
  - `0.2.0-alpha.90`, the portable ZIP of the GitHub release
    [`v0.2.0-alpha.90`](https://github.com/Thanaen/Bientot/releases/tag/v0.2.0-alpha.90)
    (commit `891383f`).
  - `0.2.62.0`, the Store-signed package installed from the Microsoft Store.
  - The fixes of [#121](https://github.com/Thanaen/Bientot/pull/121),
    built on the same PC with
    `cargo build --release --locked -p bientot-app --bin worktime --features release`.
- **How.** The session was locked from 20:10 to 20:30. During that time the
  app was driven through UI Automation (the accessibility API that screen
  readers use), keys were sent to its window with `PostMessage`, and windows
  were captured with `PrintWindow`. After the unlock, the command palette was
  driven with the real keyboard. Demo runs used `--demo <scenario>` with a
  throwaway `--data-dir`; real-data runs used `%LOCALAPPDATA%\Bientot`.

"Automated" names the tests that cover a step; the CI `Rust` job runs them
on every change, and `Rust on Windows` runs those of `bientot-windows`.
"Live" is what this run observed.

## Core scenario

The defaults of the onboarding are the configuration of the scenario: target
7:00, lunch 12:00–14:00, latest departure 18:30, orange margin 15 min,
minimum 6:00.

| Step | Result | Evidence |
| --- | --- | --- |
| 1. Active morning time is credited | Pass | Automated: `overlapping_active_evidence_produces_one_credited_timeline`, `the_day_is_recalculated_from_the_built_in_source`. Live: `--demo morning` shows « Temps travaillé : 2 h 00 min sur 7 h 00 min ». |
| 2. A lock from 12:15–13:00 is excluded as lunch | Pass | Automated: `lock_crossing_lunch_splits_into_provisional_and_excluded_segments`. Live: in the week of `--demo history-week`, Tuesday shows « Pause, 12:15–13:00 (45 min) ». |
| 3. Active time during lunch remains credited | Pass | Live: the same Tuesday credits « Travail, 08:15–12:15 », past the 12:00 start of lunch, and 13:00–15:15. |
| 4. A lock outside lunch is provisional, flagged after 90 minutes | Pass | Automated: `outside_lunch_lock_is_provisional_without_double_counting_activity`, `lock_at_threshold_creates_one_pending_review`. Live: Wednesday of `history-week` shows « À vérifier, 10:00–12:00 (2 h 00 min) » and « 2 à régler ». |
| 5. A lock past the end of the workday is excluded as `overnight-lock`, with no review | Pass (automated) | `ended_day_with_lock_open_at_window_end_excludes_it_as_overnight_lock`, `lock_carried_from_an_earlier_workday_has_no_review_to_confirm`, `overnight_lock_is_excluded_until_declared_as_work`. Not observed live: it needs a night. |
| 6. Before 14:00, the departure time shows with the lunch line | Pass | `--demo morning` at 11:00: « Départ prévu : 18:00 », « Pause déjeuner prévue jusqu’à 14:00 ». Rewritten for the D-015 revision of 23 October 2026. |
| 7. Green, orange and red match the thresholds | Pass | Automated: `forecast_colours_respect_exact_threshold_boundaries`. Live: `on-track` « Dans les temps », departure 17:00; `near-limit` « Proche de la limite », 18:20 with a 10 min margin; `late` « En retard », 19:00, 30 min past the limit. |
| 8. At target, blue, and overtime grows only with new credited time | Pass | Automated: `blue_overrides_forecast_at_and_beyond_the_effective_target`, `after_target_overtime_grows_with_newly_credited_work`. Live: `goal-reached` « Objectif atteint », « Heures supplémentaires : 1 h 00 min ». |

## Schedule settings scroll and keyboard scenario

The Rust app has no « Horaires » dialog; the eight numeric settings are on
the pages of « Paramètres ». Five have step buttons (margin, long lock, the
two notification leads, idle threshold); three are `H:mm` durations (target,
minimum, weekly debt limit).

| Step | Result | Evidence |
| --- | --- | --- |
| 1. Short window, notification leads editable | Pass | The window would not shrink below 884 × 601 pixels on this screen. At that size, « Général » scrolls, and its title and status stay in place. Both approach notifications were on, so their leads were editable; weekly debt was turned on to edit its limit. |
| 2. Step buttons stay beside their value while scrolling | Pass | Captured at the top and at the bottom of « Général »: each row keeps its control at the right of the card. |
| 3. Keyboard edit, save, reopen | Pass | Each field was focused, then: Up (spin fields) or typed text (durations), then Enter. The settings window was closed and opened again. All eight values persisted, and `settings.json` holds them: target 7:15, minimum 6:15, debt limit 2:00, margin 20 min, long lock 1:45, both leads 20 min, idle 210 s. Tab moved focus through the pages (focus ring captured). Automated: `enter_commits_and_arrows_step_a_number_field`, `tab_reaches_the_times_of_the_schedule`. |

## Weekly credit scenario

| Step | Result | Evidence |
| --- | --- | --- |
| 1. 7:30 on Monday gives +30 min | Pass (automated) | `applies_monday_overtime_to_tuesday_without_creating_new_overtime_at_the_effective_target`. |
| 2. 6:30 on Tuesday is blue with 30 min applied | Pass | Same test, and `monday_overtime_reduces_tuesday_target_and_publishes_after_the_commit`. Live: in `history-week`, Tuesday reads « 7 h 00 min comptées sur 6 h 00 min », Monday's credit applied. |
| 3. Correcting Monday recomputes Tuesday and the balance | Pass (automated) | `correcting_monday_recalculates_the_literal_week_without_mutating_prior_data`, `retrospective_monday_correction_revises_affected_days_and_reuses_unaffected_head`. |
| 4. Credit above the 6:00 floor stays until Friday | Pass (automated) | `applies_only_enough_credit_to_reach_the_floor_and_expires_the_friday_balance_prospectively`, `credit_properties.rs`. |
| 5. The Friday remainder expires on Monday | Pass (automated) | `friday_expiry_changes_once_and_following_monday_starts_at_zero`, `a_separate_week_starts_at_zero_after_friday_credit_expires`. |

## Resilience scenario

| Step | Result | Evidence |
| --- | --- | --- |
| 1. Input unreadable: degraded, no crash, no false zero | Pass | `--demo source-unavailable`: « Suivi à vérifier », the source message, « Départ prévu : Prévision indisponible », « Temps connu : 0 min ». No red state is invented. Automated: `source_outage_and_recovery_do_not_invent_work`. |
| 2. Automatic recovery | Pass (automated) | `source_failure_retains_committed_week_and_recovers`. |
| 3. Suspend while locked excludes the sleep | Pass (automated) | `suspend_wins_over_lock_and_active_evidence`. Live: Thursday of `history-week` shows « Veille, 11:00–11:40 », not credited. A real suspend was not run: it would have suspended the session doing this run. |
| 4. Start after morning activity | Pass | First real start at 20:13: the startup probes recorded the session state from the Windows logs (`SessionEvent` rows `startup-knowledge` and `power-history`), and the time before the confirmation of the onboarding is not credited (« 0 min »). Automated: `the_session_before_the_first_start_is_rebuilt_to_review`, `a_fresh_built_in_install_reads_nothing_before_onboarding_confirmation`. |
| 5. No duplicate notification after a restart | Pass (automated) | `receipts_deduplicate_across_a_coordinator_restart`. No notification was due during the run. |
| 6. Idle counts from the last input | Pass | Real data: `ActivitySpan` 3 is idle from 19:26:34 UTC, the last input, and settled at 19:29:50 when input came back. With a 180 s threshold, idle starts at the last input, not when detected. Automated: `crossing_the_threshold_backdates_idle_to_the_last_input`. |
| 7. Killed process: gap unknown. Clean exit: time after last input unknown | Pass | Killed at 19:13:32 UTC: the span stops at 19:13:03 and nothing covers the time until the next start at 19:13:54. Clean exit with « Bientôt : quitter » at 19:36:23: span 4 is settled through 19:36:20, the last input sample, and nothing covers the time until the restart at 19:36:44. Automated: `crash_with_a_pending_tail_then_restart_…`, `clean_stop_with_a_pending_tail_credits_activity_only_through_the_last_input_hours_later_…`. |
| 8. Switch from ActivityWatch to the built-in source | Pass (automated) | `switching_records_the_new_source_from_now_so_the_day_is_split_there`. Live: `--demo source-activitywatch` offers the built-in source. Only this one-time switch remains possible. |

## Release smoke scenario

Recorded in [release-checklist.md](release-checklist.md#release-smoke).

## Start-with-Windows visibility scenario

| Step | Result | Evidence |
| --- | --- | --- |
| 1. `Startup` mode | Pass | `verify-startup-visibility.ps1 -Mode Startup`: process alive, 0 show events, 0 visible windows. |
| 2. `Manual` mode | Pass after a fix of the script | The script started the app with `STARTF_USESHOWWINDOW` and `SW_HIDE`, so Windows hid the app's first window and the mode failed. Without that flag (fixed in this release), 1 show event and 1 visible window. A launch by hand shows the flyout. |
| 3. `Failure` mode | Pass after the same fix | 1 window titled « Bientôt », which shows « Bientôt n’a pas pu démarrer », the path and the OS error. |
| 4. Demo with the startup marker | Pass | `--demo --worktime-windows-startup` left the `WorkTime` Run value and its `StartupApproved` entry unchanged. |
| 5. Tray and notification activation reopen the window | Pass, tray click not run live | A second launch brings up the running copy's flyout (single instance). `test-rust-toast-activation.ps1` passes: COM starts the stopped portable build with `--toast-activated`, which logs `notification.activated` with `origin: cold`. Automated: `a_second_launch_activates_the_owner_until_it_releases_the_folder`, `a_notification_activation_remains_visible_when_the_marker_is_present`. |

## Diagnostic export smoke

| Step | Result | Evidence |
| --- | --- | --- |
| 1. Export during a startup failure | Pass | From the failure window (`--data-dir` pointing at a file): `manifest.json`, `snapshot.json` (`"initialization":"Failed"`) and an empty `logs.jsonl`. |
| 2. Export from the running app | Pass | From « Bientôt : exporter le diagnostic… » in the command palette of the Store build. The save picker suggests `bientot-diagnostics` with the « Archive ZIP » type. The tray menu item runs the same command; it was not opened (it needs a real right-click). |
| 3. Cancel | Pass | No notice in the failure window, and no file appeared. |
| 4. Contents and notice | Pass | Three entries only. « Le ZIP de diagnostic a été enregistré. » shows in the flyout and in the failure window. The ZIP was written where the picker said, and nowhere else. |
| 5. No secret, path, app or window title | Pass | Neither archive contains the user name, the machine name, a path, or the name of an open application (searched for `Utilisateur`, `pc-clem`, `AppData`, `C:\`, `Helium`, `Claude`, `Discord`, `Notepad` and a file name open in Notepad). Logs keep `startup.tray_ready` only, with minute timestamps; 2 unknown events were dropped. Automated: `writes_useful_fixed_artifacts_without_copying_hostile_values`. |
| 6. Denied destination | Pass (automated) | `preserves_existing_destination_and_cleans_owned_temporary_files`. |

## Lifecycle smoke boundary

| Check | Result | Evidence |
| --- | --- | --- |
| Lock and unlock are recorded once | Pass | Real data, Store build: the startup probes recorded `locked` at both starts during the lock, and the unlock at 19:30:02 UTC is one `unlocked` row. |
| Suspend and resume | Not run | Suspending the machine would have suspended this run. The power history of the night before was read from the System log at the first start (`suspend` 22:07:28 UTC, `resume` 18:06:33 UTC). |

## Defects found

- `scripts/verify-startup-visibility.ps1` hid the window it expected to see.
  Fixed in this release.
- Screen readers got empty names for the settings section headings, the
  version and the lunch range, and could not expand « Voir ce qui est
  envoyé ». Two colours missed WCAG AA. Fixed in
  [#121](https://github.com/Thanaen/Bientot/pull/121); see
  [accessibility-checklist.md](accessibility-checklist.md).
- After a move from the installer or the portable build to the Store, both
  copies can start at sign-in. Documented in
  [startup-registration.md](startup-registration.md#moving-to-the-store-build);
  a fix is tracked in [#124](https://github.com/Thanaen/Bientot/issues/124).

## Plugin runtime

The scenario of
[`specs/003-plugin-runtime/quickstart.md`](../specs/003-plugin-runtime/quickstart.md)
(T065), before the `plugins` feature joins the release builds.

- **Date.** 15 November 2026, 23:38 to 00:05 (Paris time).
- **Machine.** The maintainer's PC, as above. The session was unlocked, and
  nobody was in front of it.
- **Build.** `origin/main` at `d14ce90`, built on that PC with
  `cargo build --release --locked -p bientot-app --bin worktime --features release,plugins`.
  The sample is `focus`, packed and signed with a throwaway key.
- **How.** The app was driven through UI Automation, with a fresh
  `--data-dir` for every run. The data folder of the installed Bientôt was
  never opened.

| Step | Result | Evidence |
| --- | --- | --- |
| 1. Build and test the sample | Pass | `focus.wasm` is 370,704 bytes. `cargo test -p bientot-plugin-sdk --example focus`: 4 of 4. |
| 2. Pack, sign and check | Pass | The package is 113 KiB. `bientot-plugin check --expect` matches `en.txt` and `fr.txt` for the signed and the unsigned package: 15 events, at most 1.6 M fuel of 20 M, at most 1 ms each. |
| 3. Install and see the card (SC-008) | Pass | The consent dialog reads « Installer Concentration ? », the version, « Empreinte de l’auteur : a22e… », « Lire votre journée en cours », « Afficher une carte dans le flyout » and « Il ne pourra rien faire d’autre. ». Then « Concentration est installé. », and the flyout shows « Concentration » under the counted time. The 199 s measured are the automation's, mostly spent in the file picker. A demo run has no plugins: the quickstart said otherwise, and is fixed. |
| 4. Run its command | Pass | Ctrl+Maj+P: `concentration` finds « Concentration : afficher ou masquer le détail » in French, `focus` finds « Focus: show or hide the detail » in English. The command runs. With no counted time, the card has no detail to show; the change of card is covered by `the_detail_button_shows_the_sessions`. |
| 5. Screen readers (SC-007) | Fixed | UI Automation found the card's group, its « … » button and its menu, in French and in English. It did not find the card's lines, the empty page, or the version, author and capabilities of an installed plugin: they are exposed since this report. The consent dialog's text is its description (`FullDescription`), which the dump did not read. Narrator was not heard: nobody was there. |
| 6. Unsigned package | Pass | The dialog warns « L’auteur ne peut pas être vérifié, et ce plugin ne pourra jamais être mis à jour. ». The row reads « Version 0.1.0 · Non signé : l’auteur n’est pas vérifié ». Off, on, then « Retirer »: « Concentration est retiré. », and its folder is gone. |
| 7. Revocation list (T054) | Pass | After the install, `plugins\revocations.json` is byte for byte the list served at `https://bientot.dev/plugins/revocations.json`. With the request sent to a closed port, turning the plugin off and on again turns it on, keeps the last list and shows « Bientôt n’a pas pu télécharger la liste à jour des plugins bloqués. ». |
| 8. `--no-plugins` (FR-052) | Pass | No card, and the plugin stays on for the next start. The page shows « Les plugins ne sont pas disponibles. », as in a demo. |
| 9. Drop a package | Pass | A real OLE drag over **Settings › Plugins** tints the page (RGB 40,44,51 to 48,57,69) and the drop opens the consent dialog. |
| 10. Startup, memory and CPU (SC-002, SC-003) | Pass | Without a plugin: 0.31 s to `startup.tray_ready`, 0.020 % idle CPU, 55.3 MiB peak idle working set (0.32 s, 0.020 %, 55.4 MiB for a build without the feature). With the sample: 0.32 s, 0.013 %, and the working set grows by 2.5, 2.6 and 2.8 MiB over three runs, under the 3 MiB of SC-003. |
| 11. Executable size (SC-001) | Pass | 16,549,888 bytes with plugins and 14,753,792 without: 1.71 MiB more, under 2.0 MB and 20 MB. |
| 12. Hostile corpus (SC-004) | Pass | `cargo run -p bientot-plugins --example hostile-corpus --release`: 18 modules, no problem. 16 stop after three errors in 0.54 to 1.05 s, `log-flood` and `probe` live on. Probe commands: median 0.1 ms, slowest 195 ms; longest UI frame 0.03 ms. 60 log lines kept of 3,300. The working set ends 1.9 MiB above its start. |

### Defects found

- Screen readers missed the card's lines and the details of an installed
  plugin. Fixed with this report.
- Two spellings of one `--data-dir` start two primary instances on the same
  folder: [#186](https://github.com/Thanaen/Bientot/issues/186).
