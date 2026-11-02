# Release checklist

This page records the release smoke of
[`quickstart.md`](../specs/001-core-workday-indicator/quickstart.md) (T108)
and the Microsoft Store package checks (T117) for v1.0.0. The other scenarios
are in [validation-report.md](validation-report.md).

## Machine

The run of 2 November 2026 used the maintainer's PC, described in
[validation-report.md](validation-report.md#run), not a clean virtual
machine. Windows Sandbox and Hyper-V are not enabled on it, and enabling
them needs an administrator and a restart. Bientôt had never been installed
there: no package, no `%LOCALAPPDATA%\Bientot`, no Run value. Everything
Bientôt reads started empty. What a clean machine would add is a Windows
without other apps, policies or earlier settings.

## Release smoke

| Step | Result | Evidence |
| --- | --- | --- |
| 1. Install, French onboarding, no ActivityWatch step, built-in source healthy | Pass | Portable `0.2.0-alpha.90`, first start: « Choisissez votre langue » (Français), « Votre journée type » (defaults 7:00, 12:00–14:00, 18:30), « Démarrage et connexions », « Commencer ». No ActivityWatch step. The flyout shows « 0 min sur 7 h 00 min »: the time before the confirmation is unknown. The Store build then started on the same data without onboarding. Upgrading a data folder of the first version is covered by `a_legacy_dotnet_database_upgrades_as_dotnet_upgrades_it` and `a_database_migrated_by_dotnet_is_current`. |
| 2. Start-with-Windows consent and sign-in launch | Pass, sign-in not run | The onboarding toggle registered `WorkTime` = `"…\worktime.exe" --worktime-windows-startup`. In the Store build the toggle enabled `WorkTimeStartupTask` (state 2), and the status reads « Bientôt démarrera à la prochaine ouverture de session ». A sign-in was not run: signing out would have ended this run. The hidden startup is covered by `verify-startup-visibility.ps1 -Mode Startup`. |
| 3. Tray recovery after an Explorer restart | Pass | `explorer.exe` was stopped and Windows restarted it. The Bientôt icon came back in the notification area with a current tooltip, and the process kept running. |
| 4. English override and complete resources | Pass | `--english`: every name of the flyout is English (« Check activity tracking », « Known work time: 0m of 7h 00m », « Pause tracking, Ctrl+Alt+P »). Automated: `the_table_matches_the_resw_files`. |
| 5. Portable alpha limitations documented | Pass | README, « Download », and [startup-registration.md](startup-registration.md). The ZIP build greys the update setting and links to GitHub (`the_zip_build_greys_the_updates_and_links_to_github`, seen live). |
| 6. An update offer needs a user action | Pass (automated), live check pending | `nothing_is_offered_before_a_check_finds_an_update` and the `--demo-update` demo. The live check is the Store update of the installed `0.2.62.0` to 1.0.0, below. |
| 7. Diagnostics hold no secret, app or window title | Pass | [validation-report.md](validation-report.md#diagnostic-export-smoke). |
| 8. Startup under 3 s, idle CPU under 0.5 %, working set at most 150 MiB | Pass | `scripts/measure-performance.ps1 -Release` on the portable build, 900 s idle after a 5 s settle, sampled every second. |

The table of the performance run:

| Measure | Result | Budget |
| --- | --- | --- |
| Startup to `startup.tray_ready` | 0.30 s | 3 s |
| Average idle CPU | 0.013 % | 0.5 % |
| Peak idle working set | 66.3 MiB | 150 MiB |

Last working set 66.0 MiB, lifetime peak 67.8 MiB, peak private bytes
63.6 MiB, on 16 logical processors. Startup is measured from the process
creation, not from a sign-in.

## Microsoft Store package

| Check | Result | Evidence |
| --- | --- | --- |
| Install from the Store | Pass | `winget install --id 9N4FL1HRF4KL --source msstore` installed `Thanaen.Bientt` `0.2.62.0`, `SignatureKind: Store`, under `C:\Program Files\WindowsApps`. |
| Data of the portable build found | Pass | The Store build opened `%LOCALAPPDATA%\Bientot` that the portable build had created, with no onboarding, and kept writing there. `%LOCALAPPDATA%\Packages\Thanaen.Bientt_j4qejy376e90w\LocalCache\Local` has no `Bientot` folder, so writes are not virtualized. |
| Startup task | Pass | Declared disabled; the settings toggle enables it (state 2). After a reinstall, the settings show it off again, which is the real Windows state. |
| Update through the Store and the in-app offer | Pending | The live check is the update of the installed `0.2.62.0` to the 1.0.0 submission once certified. |
| Uninstall | Pass | `Remove-AppxPackage` removed the package, its `Packages` folder and the startup task; no notification registration of the package remains. `%LOCALAPPDATA%\Bientot` stays, as [privacy.md](privacy.md#delete-your-data) says. |
| Reinstall | Pass | The reinstalled package found the same history (« 23 min »). |
| Privacy policy | Pass | The Store catalog serves `PrivacyUrl` `https://github.com/Thanaen/Bientot/blob/main/docs/privacy.md` for product `9N4FL1HRF4KL`. |

## Before submitting a release

1. Bump `version` in `Cargo.toml` and `Version` in
   `packaging/msix/AppxManifest.xml` (`<version>.0`): the Store build shows
   that version, and `tests/msix_manifest.rs` checks that both agree.
2. Run the `Store listing` workflow and check that it reports no difference,
   so that no text edited in Partner Center is lost.
3. Update `ReleaseNotes` in `packaging/store/listing.json`, in English and
   French.
4. Run the `Store package` workflow with **Submit the package to the Store**,
   and approve the `release` environment.
5. After certification, update an installed copy through the Store and check
   the version in « Paramètres › À propos ».
