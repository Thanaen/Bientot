# Lots: Linux support

One row per GitHub issue, all under one epic « Linux support »
(milestone « Linux », label `linux`). The title is the title of the lot's
main pull request; a lot with several pull requests lists them in
`tasks.md`. Sizes: **S** up to two days, **M** up to a week, **L** one to
two weeks, **XL** more, for one developer, reviews and real-desktop
validation included. They are estimates made before any Linux backend
exists.

Every lot leaves Windows shippable: its pull requests pass `Rust` and
`Rust on Windows`.

| Lot | Title | Goal | Tasks | Depends on | Verification | Size | Needs owner decision |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | `docs(repo): ratify platform parity in the constitution and D-026` | The project's rules name two platforms; the accounting decision for Linux evidence is recorded | T001–T004 | — | Constitution at 1.3.0 with Principle VII; D-026 in `docs/product-decisions.md`; `docs/platforms.md` exists | S | **Yes**: Q1–Q14, above all Q4 and Q9 |
| 1 | `refactor(app): move the portable platform logic into bientot-platform` | `bientot-windows` holds only Windows code; history ports return boundaries, not Event Log records | T010–T013 | — | No test body edited; ledger fixtures byte-identical; Windows executable within 50 KB; `Rust on Windows` green with package and toast scripts | L | No |
| 2 | `refactor(app): build the app on platform ports from one composition root` | `bientot-app` has no platform condition outside `src/platform/`; CI refuses new ones | T020–T026 | 1, 0 | Screenshots identical to `main`; gate script fails on a seeded violation; manual Windows pass of `quickstart.md` | L | No |
| 3 | `ci(repo): build, measure and start Bientôt on Linux in every pull request` | Both platforms are compiled, linted and tested on every pull request; a Linux executable is measured and started headless | T030–T037 | 2 | `Rust on Linux` green: size reported against 20 MB (a gate once issue #226 lands), headless start, hostile corpus; Windows cross-lint in `Rust`; no `libssl` after T037 | M | Add `Rust on Linux` to required checks (T036) |
| 4 | `build(app): add the Linux crate and a tray and flyout spike` | Assumptions A-1 to A-5 become facts on real desktops before anything is built on them | T040–T046 | 3 | « Spike results » in `research.md` for Cinnamon, GNOME, KDE, Sway; contracts and parity matrix corrected | M | Only if a Tier 1 desktop fails with no fallback (T046) |
| 5 | `build(app): show the tray icon and its menu on Linux` | The icon, tooltip and menu of Windows, on every desktop with a tray host | T050–T056 | 4 | Harness tests against a test host; Tier 1 pass (T055); icon approved by the panel | M | No (panel review) |
| 6 | `build(app): place the flyout on X11 and Wayland` (+ `build(app): run in a window when the desktop has no tray`) | The flyout opens where the parity matrix says; a desktop without a tray gets window mode | T060–T066 | 5 | Harness: bounds under `openbox`, layer surface under `sway`; steps 4–10 of `quickstart.md` on Tier 1; panel approval of frame and notice | L | Q5 |
| 7 | `build(app): detect activity on Linux` | Idle time read on every Tier 1 desktop by the same rules as Windows; said plainly where it cannot be | T070–T077 | 3 | Golden ledger identical to the Windows path; harness per backend; steps 11–13 on Tier 1 | L | Q12 |
| 8 | `build(app): record lock, unlock, sleep and resume on Linux` | The lifecycle evidence of Windows, from logind | T080–T086 | 3 | Golden ledger identical to the Windows decoder; dbusmock tests; steps 14–17 on Tier 1 | M | No |
| 9 | `build(app): recover power and session history on Linux` | Past sleep recorded when readable; « Avant Bientôt » by D-026 | T090–T095 | 8, 0 | Fixture tests from real journals; steps 18–19 on Ubuntu and Fedora; panel approval of the review text | M | **Yes**: Q4 blocks the lot |
| 10 | `build(app): show reminders as desktop notifications on Linux` | Reminders with the Windows texts; click opens the flyout | T100–T104 | 3 | dbusmock tests; steps 20–21 on GNOME, KDE, Cinnamon | S | No |
| 11 | `build(app): start at sign-in and keep one instance on Linux` (+ `build(app): name the Linux executable and data folder`) | Autostart with truthful states; one instance per data folder; Linux names | T110–T117 | 3 | File-state tests; two-process test; steps 22–24; panel approval of settings and onboarding | M | Q11 |
| 12 | `build(app): give Linux its own wording` (+ `fix(app): read exports from the other platform`, `feat(app): follow the system language`) | No text names the wrong platform; data crosses platforms; language, motion, diagnostics, plugins, screen reader | T120–T129 | 3 | SC-012 string test; cross-platform fixture test (SC-010); steps 25–29; screenshots for both platforms in CI | L | No (panel review); the `feat` ships to Windows |
| 13 | `build(app): add the Linux desktop files and the Flatpak manifest` (+ `ci(repo): build the Flatpak and open the Flathub update`) | Bientôt on Flathub, verified, x86_64 and aarch64 | T130–T137 | 5–12 | `flatpak-builder-lint` clean; sandbox pass on Tier 1 (T131); Flathub review accepted | L | **Yes**: Q2 (app id, irreversible), Q13; Flathub account (T136) |
| 14 | `build(app): package the AppImage with Velopack` (+ `build(app): update the AppImage from the app`) | A single file that runs on glibc 2.35+ and updates itself | T140–T147 | 5–12 | glibc and size gates; headless start of the AppImage in CI; step 30 with a real pre-release | L | Q8 (aarch64 hardware) |
| 15 | `ci(repo): release Windows and Linux together` (+ `feat(site): serve the Linux release files`) | One release, one version, both platforms, attested; notes per platform | T150–T155 | 14 | A draft release holds every file of `contracts/packaging.md` with attestations; script tests | M | **Yes**: Q6, Q7 |
| 16 | `feat(site): accept the Linux channels` (+ `docs(repo): describe the usage signal on Linux`, `build(app): send the usage signal from Linux`) | The usage signal counts Linux installs; nothing leaves that the privacy page does not list | T160–T164 | 11 (install kind) | Service and app tests of `contracts/usage-signal.md`; a Windows body byte-identical | S | **Yes**: Q3 |
| 17 | `feat(site): offer Bientôt for Linux` | The site offers the right download and explains the GNOME tray | T170–T174 | 15 | Route tests; panel approval of the page and help text | M | No (panel review) |
| 18 | `feat(app): ship Bientôt for Linux` (+ `docs(repo): document Bientôt on two platforms`) | Everything validated on Tier 1; documents for two platforms; the launch | T180–T186 | all | 30 steps × 4 desktops × 2 packages recorded; every assumption closed; SC-001 to SC-012 met | L | **Yes**: launch date and announcement |

Order of work, by risk: 0 and 1 at once; 2; 3; then 4 (the spike) before
anything visible; 7 and 8 beside 5 and 6; 9 when Q4 is answered; 10, 11,
12; 13 and 14; 15, 16, 17; 18.

## Open questions for the owner

Each has a recommended answer; the plan is written for it. « Blocks » says
what cannot start without the answer.

**Q1. Which packages are official at launch?**
Recommended: **Flathub and an AppImage**, nothing else. Flathub is the
store (reviewed, updated by the system, in most software centres); the
AppImage is the single file with every feature and nightly self-update.
They mirror Store and `Setup.exe`. Blocks: Lots 13, 14.

**Q2. Is the app id `dev.bientot.Bientot`?**
Recommended: **yes**. It follows the domain the project owns, which
Flathub verifies. It cannot change after publication. Blocks: Lot 13 (and
the file names of Lots 10, 11).

**Q3. May Linux builds add `desktop` and `session` to the usage signal?**
Recommended: **yes, those two closed lists and nothing more**
(`contracts/usage-signal.md`), documented before the code sends them. Five
desktops by two session types identify nobody and tell where to test. If
no: only the two new channel values. Blocks: Lot 16.

**Q4. What does « Avant Bientôt » rebuild on Linux, where past locks are
never recorded?**
Options: (a) nothing, strict application of the Windows rule; (b) from the
session opening to the first start, minus known sleep, as one provisional
review that says locks are unknown; (c) the same, but offered unchecked
(counted only if the user ticks it).
Recommended: **(b)**, recorded as D-026. It is what the Windows review
already is (provisional, reviewed), with the limit said in words. Blocks:
Lot 9 and T003.

**Q5. Without a tray host (upstream GNOME), is window mode the right
fallback?**
Options: (a) window mode with a notice and a help page; (b) refuse to
start and explain; (c) ship a GNOME Shell extension.
Recommended: **(a)**. Bientôt counts the day either way, and the user
keeps a way in. (c) is a second product to maintain against every GNOME
release. Blocks: PR 6b.

**Q6. Does a failed Linux build block the night's release for Windows, and
are Linux files published before the launch?**
Recommended: **yes, one release or none**, so a version number always
means the same code on both platforms; and **yes, as « preview »** from
Lot 15, unlisted on the site until T186, so the update path is exercised
for weeks before anyone is told. Blocks: Lot 15.

**Q7. May a release-summary line concern one platform?**
Recommended: **yes**, with an optional `platforms:` line; notes print
common lines, then « Windows », then « Linux ». Until the launch, Linux
pull requests are `build(app)` and owe no line. Blocks: T152, T153.

**Q8. Which machines validate Tier 1, and is aarch64 official?**
Recommended: Tier 1 as in `spec.md` (Ubuntu GNOME, Fedora GNOME, KDE
Plasma 6, Mint Cinnamon); virtual machines are acceptable except for
suspend and two-monitor checks, which need one real machine; **aarch64
ships as Tier 2** unless the owner has such a machine. Please say what
hardware you have. Blocks: every **[DESKTOP]** task.

**Q9. Do you ratify constitution 1.3.0 (Principle VII, Platform parity)
and the three amended texts in `plan.md`?**
Recommended: **yes**. Blocks: Lot 0, then the gate of Lot 2.

**Q10. `.deb`, `.rpm`, Snap, AUR?**
Recommended: **none maintained by the project at launch**; say in
`docs/platforms.md` that community packages are welcome and may wrap the
AppImage. Revisit when the usage signal shows where users are. Blocks:
nothing.

**Q11. Is the Linux executable called `bientot`?**
Recommended: **yes**. `worktime` is the old product name; Windows keeps
`worktime.exe` because installed copies, the Store package and scripts
depend on it, but nothing on Linux does yet, and the name shows in process
lists, autostart entries and the desktop file. Blocks: T114, and file
names in Lots 13, 14.

**Q12. On a compositor that only offers version 1 of the idle protocol, an
app that inhibits idle (a video call) makes the user look active. Accept?**
Recommended: **accept, logged and documented**: counting a call as
activity is the lesser error, Tier 1 desktops are expected to offer
version 2 or another backend (T072 verifies), and the alternative is no
detection at all there. Blocks: nothing; changes one row of the matrix.

**Q13. Flathub cadence: weekly with the Store, built by Flathub from
source?**
Recommended: **yes**. Flathub builds from the tagged source with vendored
crates, which its reviewers prefer to a prebuilt binary; weekly matches
the Store and keeps nightly churn out of users' software centres. Blocks:
T135.

**Q14. Signing: attestations only, no GPG signature of the AppImage?**
Recommended: **yes**, as on Windows today: SLSA provenance for every file,
Velopack's package hashes for updates, Flathub's own signature for the
Flatpak. A GPG signature that nothing verifies at run time adds a key to
guard and no protection. Blocks: nothing.
