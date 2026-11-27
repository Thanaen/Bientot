# Feature Specification: Linux support

**Feature Branch**: `005-linux-support`

**Created**: 2026-11-24

**Status**: Draft, waiting for the owner's answers to the open questions of
`lots.md` before planning review

**Input**: The owner asked for Bientôt on Linux, as a platform of the same
standing as Windows: « Linux n'est pas une plateforme secondaire », both
platforms have the same importance, and the work is to be done properly,
with a specification. This document says what « the same product on Linux »
means, feature by feature, and what it cannot mean on some desktops.

## Background: where the code stands

Checked on `main` at `d9f55ce` (`research.md`, R1 and R2):

1. **The workspace already compiles on Linux**, because every Windows call
   sits behind `cfg(windows)`. CI lints and tests it on Ubuntu.
2. **What it builds there does nothing.** No tray, no activity detection,
   no lock or sleep events, no notification, no update. The process starts
   and counts no time.
3. **Most of the code is not Windows code.** The rules, the ledger, the
   settings, the plugin runtime and every screen are portable. About 2 400
   lines call Win32 or WinRT; about 7 000 portable lines live in a crate
   named `bientot-windows`.
4. **Linux is several desktops.** A tray exists on KDE Plasma and Cinnamon
   and, on GNOME, only with an extension. A program can place its window on
   X11, in a corner on KDE's Wayland, and not at all on GNOME's Wayland. No
   single interface reports idle time. Nothing records past screen locks.

## Scope

In scope:

- Every feature of the Windows app on Linux, on the desktops listed below,
  with each difference named in the parity matrix.
- The structure that keeps both platforms compiled and tested on every pull
  request.
- Two official packages (Flathub, AppImage), for x86_64 and aarch64, in the
  same release as Windows.
- The site, the usage signal, the documentation and the release notes for
  two platforms.

Out of scope:

- macOS, BSD, ChromeOS containers, WSLg.
- `.deb`, `.rpm`, Snap and AUR packages maintained by the project (Q10).
- A GNOME Shell extension or a KDE plasmoid shipped by Bientôt.
- A setting for the flyout's corner.
- Reading the foreground application or window title (not read on Windows
  either).
- A secret store on Linux: nothing is stored that needs one.
- Sharing one data folder between the Flatpak and the AppImage.
- Translations beyond French and English.

## Supported desktops

**Tier 1**: tested by hand before a release that touches the Linux shell;
a defect there blocks the release.

| Distribution | Desktop | Session | Why |
| --- | --- | --- | --- |
| Ubuntu 24.04 LTS | GNOME 46 with Ubuntu's AppIndicator extension | Wayland | The most used desktop distribution; the only GNOME that ships a tray host |
| Fedora Workstation, current | GNOME, no extension | Wayland | Upstream GNOME: proves the no-tray path (FR-012) and the newest Mutter |
| Fedora KDE or Kubuntu, current | KDE Plasma 6 | Wayland | Second desktop by use; tray, layer-shell and idle protocol all present: the reference for « same as Windows » |
| Linux Mint 22 | Cinnamon | X11 | The most used X11 desktop; proves the X11 path |

**Tier 2**: expected to work; defects are fixed but do not block a release.
Debian 12 and 13, Arch, openSUSE Tumbleweed; Xfce, MATE, Budgie, LXQt
(X11); Sway, Hyprland, niri, COSMIC (Wayland); every aarch64 machine.

**Not supported**: glibc older than 2.35 (Ubuntu 20.04, Debian 11), musl
distributions for the AppImage (the Flatpak runs there), desktops with no
session D-Bus.

The AppImage needs glibc 2.35. The Flatpak brings its own runtime and runs
wherever Flatpak does.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Windows stays exactly as it is (Priority: P1)

Camille uses Bientôt on Windows. The Linux work reorganizes the code under
her app. She must notice nothing: same tray, same flyout, same counts, same
notifications, same update.

**Why this priority**: every later lot rests on the reorganization, and a
regression on the platform that has users costs more than any Linux feature
gains.

**Independent Test**: on the commit after Lot 2, run the Windows section of
`quickstart.md` and compare ledger fixtures and screenshots with `main`.

**Acceptance Scenarios**:

1. **Given** the same recorded timeline of input, lock and sleep, **When**
   the build before and the build after the reorganization process it,
   **Then** the ledger rows are identical.
2. **Given** the screenshot scenes, **When** rendered before and after,
   **Then** every image is identical.
3. **Given** an installed copy from before, **When** it updates across the
   reorganization, **Then** it starts at sign-in, keeps its settings and
   data, and shows no new prompt.
4. **Given** any later Linux lot, **When** it merges, **Then** the Windows
   executable differs only by shared code that both platforms use.

---

### User Story 2 — A day counted on Linux (Priority: P1)

Noé works on Fedora KDE. He installs Bientôt, sees the icon in the panel
with the time left, and clicks it to see his day. Bientôt counts while he
types, stops when he walks away or locks the screen, and resumes when he
comes back, by the same rules as on Windows.

**Why this priority**: this is the product. Without it nothing else on
Linux matters.

**Independent Test**: on each Tier 1 desktop, work, idle past the
threshold, lock, suspend, resume; compare the day with the expected
timeline.

**Acceptance Scenarios**:

1. **Given** a desktop with a tray host, **When** Bientôt starts, **Then**
   its icon shows in the panel with the remaining time as on Windows, and
   a primary click opens the flyout.
2. **Given** KDE Plasma on Wayland or any X11 desktop, **When** the flyout
   opens, **Then** it sits in a corner of the screen at the panel's side,
   12 px from the edges, and closes on Esc, on a click elsewhere, or on a
   second click on the icon.
3. **Given** GNOME on Wayland, **When** the flyout opens, **Then** it
   shows the same panel where the desktop places new windows, and closes
   the same three ways.
4. **Given** no keyboard or pointer input for longer than the idle
   threshold, **When** input resumes, **Then** the idle span is treated by
   the same rule as on Windows, with the same thresholds.
5. **Given** the screen locks and unlocks, or the computer sleeps and
   wakes, **When** the day is recalculated, **Then** the lock and the sleep
   appear in the ledger with the same meaning as on Windows.
6. **Given** a secondary click on the icon, **When** the menu opens,
   **Then** it holds the same entries in the same order as on Windows.

---

### User Story 3 — Bientôt says what this desktop cannot tell it (Priority: P1)

Lina uses upstream GNOME, which has no tray, or Sway with a locker that
reports nothing. Bientôt does not pretend. It tells her what is missing,
what it does instead, and never counts time it cannot justify.

**Why this priority**: Principle II. A wrong count on Linux would be worse
than no Linux build.

**Independent Test**: start Bientôt on Fedora GNOME without extension, and
on Sway with `swaylock`; under the CI harness, start it with no idle
backend.

**Acceptance Scenarios**:

1. **Given** no tray host, **When** Bientôt starts, **Then** it opens as a
   small ordinary window, says once that this desktop shows no tray icons
   with a link to the help page, and stays reachable from the app grid.
2. **Given** window mode, **When** a tray host appears, **Then** the icon
   shows and the window becomes the flyout, without a restart.
3. **Given** no way to read idle time, **When** the day runs, **Then**
   nothing is counted automatically, the flyout says that activity cannot
   be detected on this desktop, and manual declarations work.
4. **Given** a desktop that reports no lock state, **When** the screen
   locks, **Then** the day is accounted from activity alone and the
   diagnostic export says that lock state is unavailable.
5. **Given** any of these states, **When** the user exports diagnostics,
   **Then** the file names the desktop, the session type and the backend
   used or missing for tray, idle, lock and history.

---

### User Story 4 — The comforts of every day (Priority: P2)

Noé gets the end-of-day reminder as a desktop notification, Bientôt starts
when he signs in, a second launch brings the first one forward, the
language and the light or dark theme follow his desktop, and his plugins
run.

**Why this priority**: each is a feature Windows users have; none blocks
counting a day.

**Independent Test**: each scenario alone on one Tier 1 desktop.

**Acceptance Scenarios**:

1. **Given** a reminder is due, **When** the desktop has a notification
   service, **Then** a notification shows with the Windows text, and a
   click on it opens the flyout while Bientôt runs.
2. **Given** no notification service, **When** a reminder is due, **Then**
   the flyout says notifications are unavailable and the reminder is not
   recorded as shown.
3. **Given** « start at sign-in » is on, **When** the user signs in,
   **Then** Bientôt starts hidden in the tray.
4. **Given** Bientôt runs, **When** it is launched again on the same data
   folder, **Then** the running instance opens its flyout and the second
   process exits.
5. **Given** the AppImage file was moved, **When** the settings open,
   **Then** the startup state says the registration points elsewhere and
   offers to repair it.
6. **Given** a plugin installed on Windows, **When** the same file is
   installed on Linux, **Then** it runs with the same limits and the same
   result.

---

### User Story 5 — Install and stay up to date (Priority: P2)

Lina installs Bientôt from her software centre (Flathub) and gets updates
with her other apps. Noé prefers a single file: he downloads the AppImage,
and Bientôt updates itself as the Windows installer build does.

**Why this priority**: without a package there is no user; without updates
there is no second week.

**Independent Test**: install each package on a clean Tier 1 machine;
publish a newer version; observe the update.

**Acceptance Scenarios**:

1. **Given** Flathub, **When** the user installs Bientôt, **Then** it
   appears in the app grid with its name and icon, starts, and every
   feature of the parity matrix's Flatpak column works.
2. **Given** the Flatpak, **When** the settings open, **Then** there is no
   update section, as for a Store install.
3. **Given** the AppImage in the user's home, **When** a newer release
   exists, **Then** Bientôt offers it under the same setting and wording
   as the Windows installer build, replaces its own file and restarts.
4. **Given** the AppImage on first run, **When** it starts, **Then** it
   adds its launcher entry and icon for the user, and removes them when
   asked in the settings.
5. **Given** the site on a Linux browser, **When** the user opens the
   download page, **Then** Flathub and the AppImage for their architecture
   are offered first and Windows second.

---

### User Story 6 — Change computer, change system (Priority: P2)

Camille moves from Windows to Linux, or uses both. She exports on one and
imports on the other; her history, settings and plugins arrive intact.

**Why this priority**: data that cannot follow its owner contradicts
Principle I.

**Independent Test**: export a real Windows data folder, import on Linux,
compare the week view; then the reverse.

**Acceptance Scenarios**:

1. **Given** a `.bientot` file from Windows, **When** imported on Linux,
   **Then** every past day shows the same figures, in the same time zone.
2. **Given** a `.bientot` file from Linux, **When** imported on Windows,
   **Then** the same holds.
3. **Given** an imported history, **When** the next day runs, **Then** the
   weekly credit and debt continue from it.

---

### User Story 7 — The time before the first start (Priority: P3)

On the first start in a day already under way, Bientôt offers what it can
establish about the hours before, as a review to confirm, and says what it
cannot know. After a night's sleep or a reboot, it records them.

**Why this priority**: valuable on day one, absent on no day's accounting
since the time is otherwise offered for manual declaration.

**Independent Test**: first start at mid-day on Ubuntu and on Fedora, with
and without the Flatpak sandbox.

**Acceptance Scenarios**:

1. **Given** a readable system journal, **When** Bientôt starts after the
   computer slept while it was not running, **Then** the sleep is recorded
   with its real start and end.
2. **Given** no readable journal, **When** Bientôt starts, **Then** the
   current boot is recorded, earlier sleep is unknown, and the unknown time
   is offered for manual declaration.
3. **Given** a first start with a known session opening, **When** the
   flyout opens, **Then** « Avant Bientôt » offers the time since the
   session opened, minus known sleep, as provisional, with a text that says
   locked time is not known on Linux (decision D-026).
4. **Given** an unknown session opening, **When** Bientôt first starts,
   **Then** nothing is rebuilt.

---

### User Story 8 — A contributor cannot break the other platform (Priority: P3)

Sacha fixes a bug on Linux without a Windows machine. The pull request
tells them, before review, whether Windows still compiles, lints and
passes its tests; and the reverse for a Windows contributor.

**Why this priority**: it is what keeps the two platforms equal after the
launch.

**Independent Test**: open a pull request that breaks a Windows-only line
from Linux, and one that adds a platform condition in shared code.

**Acceptance Scenarios**:

1. **Given** a change that breaks Windows-only code, **When** the pull
   request runs, **Then** a required check fails on the Ubuntu job, without
   waiting for the Windows job.
2. **Given** a platform condition added outside the platform code,
   **When** the pull request runs, **Then** a required check fails and
   names the file and line.
3. **Given** a change to shared code, **When** the pull request runs,
   **Then** the tests of both platforms run.

### Edge Cases

- **The tray host restarts** (shell crash, `plasmashell --replace`): the
  icon comes back by itself; window mode does not flicker in for a host
  that returns within 5 s.
- **Several monitors with different scales**: Wayland scales per monitor;
  X11 has one scale for all (a GPUI limit), so the flyout may be larger or
  smaller than native on a mixed X11 setup.
- **The panel is on another edge or auto-hides**: on X11 the work area
  accounts for it; on KDE Wayland the flyout uses the default corner.
- **X11 without a compositing manager**: the flyout has square corners and
  no shadow.
- **The session bus restarts or the compositor is replaced**: backends are
  selected again; the gap is unknown time.
- **A video call inhibits idle** on a compositor with version 1 of the idle
  protocol only: the user may look active while away (parity matrix).
- **Remote sessions** (RDP, VNC): the session may be reported as not
  active; treated as a lock, as Windows treats a disconnect.
- **Switch User**: the session becomes inactive; treated as a lock.
- **No systemd-logind and no elogind**: no lock or sleep events; session
  state unknown; sleep detected at the next sample as a gap of unknown
  time, never as work.
- **The clock changes** or the time zone changes while running: as on
  Windows.
- **Flatpak and AppImage both installed**: two data folders, two
  instances; the settings of each say which install it is; data moves by
  export and import.
- **An AppImage on a read-only or root-owned location**: the update asks
  for elevation through the system's prompt or reports that it cannot
  update.
- **No Vulkan driver** (some virtual machines): behaviour set by task T031;
  at least a clear message, never a silent blank window.
- **A Windows export with Windows time-zone names** imported on Linux: days
  keep their zone.
- **`$XDG_RUNTIME_DIR` unset**: the single-instance socket falls back to
  the data folder.
- **Reduced motion or high contrast set in the desktop**: followed where
  the desktop exposes it.
- **Screen reader (Orca)**: the flyout, the settings and the week are read
  with the same names as with Narrator.

## Requirements *(mandatory)*

### Functional Requirements

**Parity and structure**

- **FR-001**: The Windows app MUST behave identically before and after the
  code reorganization: same ledger rows for the same events, same
  screenshots, same package checks.
- **FR-002**: Every feature of the Windows app MUST exist on Linux with the
  same rules, texts and layout, except where the parity matrix says
  otherwise; each exception MUST state what the user sees.
- **FR-003**: Settings, ledger, transfer file and plugin files MUST have
  one format for both platforms and move between them in both directions
  without loss.
- **FR-004**: The supported distributions and desktops MUST be published
  with their tier, and each release MUST be validated on Tier 1.

**Tray and flyout**

- **FR-005**: On a desktop with a tray host, Bientôt MUST show its icon
  with the remaining time, a tooltip and the same menu as on Windows; a
  primary click MUST open the flyout.
- **FR-006**: The flyout MUST open at the panel's side on X11 and on
  Wayland compositors that let an app anchor a surface; elsewhere it MUST
  open where the compositor places it, with the same content and size.
- **FR-007**: The flyout MUST close on Esc, on loss of focus and on a
  second click on the icon, on every supported session.
- **FR-012**: Without a tray host, Bientôt MUST run in a minimizable
  window, say once why with a link to help, stay reachable from the app
  launcher, and switch to and from the tray without a restart when a host
  appears or disappears.

**Activity, session, power**

- **FR-008**: Activity MUST be detected from keyboard and pointer idle time
  alone, with the thresholds and rules of Windows, through the interface
  the desktop offers; the choice MUST be automatic and recorded in the
  diagnostics.
- **FR-009**: When no idle interface answers, Bientôt MUST count nothing
  automatically, say so in the flyout, and keep manual declarations.
- **FR-010**: Lock, unlock, sleep, resume and the session becoming
  inactive MUST be recorded with the meaning they have on Windows.
- **FR-011**: When the desktop reports no lock state, the session state
  MUST be unknown, the accounting MUST rest on activity alone, and the
  diagnostics MUST say so.
- **FR-013**: At each start, past sleep and shutdown MUST be recorded when
  the system lets the user read them, and left unknown otherwise; time is
  never invented.
- **FR-014**: The first-start rebuild MUST follow decision D-026 and MUST
  always be a review the user confirms.
- **FR-034**: Bientôt MUST NOT need elevated rights, group membership, or
  access to input devices, and MUST NOT read what is typed, clicked or
  shown.

**Everyday integration**

- **FR-015**: Reminders MUST be desktop notifications with the Windows
  texts; a click MUST open the flyout while Bientôt runs; an absent or
  refusing service MUST be reported as on Windows.
- **FR-016**: « Start at sign-in » MUST be offered with truthful states
  for each kind of install, and MUST start Bientôt hidden.
- **FR-017**: One instance MUST run per data folder; a second launch MUST
  hand its request to the first.
- **FR-018**: All data MUST live under one folder, in the user's data
  directory, shown in the settings and openable from there.
- **FR-019**: No text shown on Linux names Windows, and no text shown on
  Windows names Linux; texts that differ are listed in
  `contracts/copy.md`.
- **FR-020**: Light or dark theme and reduced motion MUST follow the
  desktop where it exposes them; the interface language MUST be resolved
  the same way on both platforms.
- **FR-021**: Plugins MUST install, run, be limited and be revoked the
  same way on both platforms.
- **FR-032**: Keyboard use and screen-reader names MUST match Windows.

**Distribution**

- **FR-022**: Bientôt MUST be published on Flathub under a verified
  listing, in French and English.
- **FR-023**: Bientôt MUST be published as an AppImage that updates
  itself under the existing update setting.
- **FR-024**: Both packages MUST exist for x86_64 and aarch64; the
  AppImage MUST start on glibc 2.35 and later.
- **FR-025**: The Flatpak MUST NOT update itself or show update settings.
- **FR-028**: The site MUST offer the right download for the visitor's
  platform and always show the other.
- **FR-029**: One release MUST carry both platforms under one version;
  release notes MUST mark lines that concern one platform.

**Trust and quality**

- **FR-026**: No information leaves the computer on Linux that does not
  on Windows, except the fields the owner accepts in Q3, which MUST be
  listed in the privacy document before the code sends them.
- **FR-027**: The diagnostic export MUST name the distribution, desktop,
  session type, install kind and each backend in use, and nothing personal
  beyond what the Windows export holds.
- **FR-030**: Every pull request MUST compile, lint and test both
  platforms, and MUST be refused when it adds a platform condition outside
  the platform code.
- **FR-031**: The Linux executable MUST stay under 20 MB, start in under
  1 s and rest under 60 MB of memory, measured as on Windows.
- **FR-033**: README, contributor guide, privacy and help MUST describe
  both platforms.

### Key Entities

- **Platform**: Windows or Linux; fixes which backends exist.
- **Desktop session**: the desktop environment and session type (X11,
  Wayland) found at start; fixes which backend each port uses.
- **Install kind**: Store, installer, portable, Flatpak, AppImage or
  development build; fixes updates, autostart mechanism and what the
  sandbox allows.
- **Backend**: one way to provide a capability (tray, placement, idle,
  lock, history, notification, autostart); named in the diagnostics.
- **Capability state**: available, or unavailable with a reason the user
  is shown.
- **Tier**: the support level of a distribution and desktop.

## Parity matrix

« Same » means the acceptance scenarios of Windows pass unchanged. Flatpak
differences are in the last column; an empty cell there means « as the
session's column ».

| Feature | Windows | Linux, X11 (Cinnamon, Xfce…) | Wayland, GNOME | Wayland, KDE Plasma | Flatpak differs? |
| --- | --- | --- | --- | --- | --- |
| Accounting rules, credit, debt, week | Reference | Same | Same | Same | |
| Settings, onboarding, week, palette, adjustments | Reference | Same | Same | Same | |
| Tray icon with remaining time | Notification area | Same (StatusNotifierItem) | **Different**: needs a tray extension (present on Ubuntu). Without one, **unavailable**: GNOME shows no tray icons; Bientôt runs in a window and says so (FR-012) | Same | |
| Icon colours | Follow the taskbar theme | **Different**: the panel's colour cannot be read; one icon with an outline that reads on light and dark | As X11 | As X11 | |
| Tooltip | Yes | Same where the host shows tooltips | Depends on the extension | Same | |
| Primary click opens the flyout | Yes | Same | Same with Ubuntu's extension; hosts that only show a menu get « Ouvrir Bientôt » as first entry | Same | |
| Secondary-click menu | Yes | Same | Same | Same | |
| Flyout position | Corner of the work area, clicked monitor | Same | **Different**: Wayland on GNOME gives apps no say; opens where the desktop places new windows, usually the centre | Same corner logic; **different** when the panel was moved and the host gives no click position: default corner (bottom right) | |
| Flyout closes on Esc, focus loss, second click | Yes | Same | Same | Same | |
| Rounded corners and shadow | Drawn by Windows | Drawn by the app; **square, no shadow** without a compositing manager | Drawn by the app | Drawn by the app | |
| Per-monitor scale | Yes | **Different**: one scale for all monitors (toolkit limit) | Same | Same | |
| Activity from input idle time | `GetLastInputInfo` | Same (XScreenSaver extension) | Same (Mutter idle monitor) | Same (idle protocol v2). With v1 only: **different**, an app that inhibits idle makes the user look active | |
| No idle interface | Never happens | Unavailable on exotic setups: nothing counted automatically, said in the flyout (FR-009) | — | — | |
| Lock and unlock, live | Session notifications | Same where the desktop sets the lock hint (Cinnamon, Xfce with its locker); otherwise **unavailable**: state unknown, activity alone (FR-011) | Same | Same | |
| Sleep and resume, live | Power broadcasts | Same (logind) | Same | Same | |
| Switch User, remote disconnect | Counted as a lock | Same (session inactive) | Same | Same | |
| Power history at start | System log | Same when the user may read the journal; otherwise **different**: current boot only, earlier sleep unknown | As X11 | As X11 | **Different**: no journal in the sandbox; current boot only |
| « Avant Bientôt » at first start | Rebuilt from logon, lock, unlock | **Different**: Linux records no past lock. From the session opening minus known sleep, as a review that says so (D-026) | As X11 | As X11 | Same as the session's column, without past sleep |
| Reminders | Toasts | Same (notification service) | Same | Same | Through the portal |
| Click on a reminder while running | Opens the flyout | Same | Same | Same | |
| Click on an old reminder after Bientôt quit | Restarts Bientôt | **Different**: depends on the notification service; may do nothing | As X11 | As X11 | |
| Start at sign-in | Run key or Store task | Same (autostart entry) | Same | Same | Through the portal; the desktop may ask once |
| « Open Startup apps » link | Yes | **Unavailable**: no common settings page; hidden | Hidden | Hidden | |
| Startup disabled by the system or by policy | Shown | **Unavailable**: no such state exists | — | — | |
| Single instance, second launch opens the flyout | Yes | Same | Same | Same | |
| In-app updates | Store: prompt; installer: self-update; ZIP: none | AppImage: same as installer | As X11 | As X11 | **Different**: none, like the ZIP; the software centre updates |
| Data folder | `%LOCALAPPDATA%\Bientot` | `~/.local/share/bientot` | Same | Same | `~/.var/app/dev.bientot.Bientot/data/bientot` |
| Store and installer share data | Yes | **Different**: Flatpak and AppImage do not; export and import | — | — | |
| Export, import, `.bientot` across platforms | Yes | Same | Same | Same | Through the file chooser |
| Diagnostic export | Yes | Same, plus desktop, session type, backends | Same | Same | |
| Plugins | Yes | Same | Same | Same | |
| Language follows the system | First display language that is French or English, else English | Same rule on `LC_ALL`, else `LC_MESSAGES`, else `LANG` (T126) | Same | Same | |
| Light and dark theme | Follows Windows | Same (portal) | Same | Same | |
| Reduced motion | Follows « Animation effects » | Same where the desktop exposes it | Same | Same | |
| Screen reader | Narrator | Orca, same names (to validate, T127) | Same | Same | |
| Usage signal | 3 fields | Same fields, channel `appimage` (Q3 may add two) | Same | Same | Channel `flatpak` |
| Demo mode, `--data-dir`, safe mode | Yes | Same | Same | Same | |
| Architectures | x64 | x86_64, aarch64 | Same | Same | Same |

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For 100 % of the recorded reference timelines (input, lock,
  sleep), the Linux build and the Windows build write identical accounting
  rows.
- **SC-002**: After the reorganization, 100 % of existing Windows tests
  pass unedited (imports aside) and 100 % of screenshot scenes are
  pixel-identical.
- **SC-003**: On each Tier 1 desktop, a user installs Bientôt and sees the
  remaining time of their day in under 2 minutes, without a terminal.
- **SC-004**: On each Tier 1 desktop, the 30-step validation of
  `quickstart.md` passes before the launch and before every release that
  touches the Linux shell.
- **SC-005**: In a full working day on each Tier 1 desktop, Bientôt's count
  differs from a hand-kept timeline by less than 2 minutes, the tolerance
  the 5-second sampling already gives on Windows.
- **SC-006**: On a desktop where a capability is missing, 100 % of the
  missing capabilities are named in the interface or the diagnostic export,
  and 0 minute is counted from a source that did not answer.
- **SC-007**: The Linux executable is under 20 MB, shows its ready state in
  under 1 s and rests under 60 MB on the reference machine.
- **SC-008**: 100 % of pull requests run the lint of both targets and the
  platform-condition gate; a seeded violation of each kind is refused.
- **SC-009**: A release publishes Windows and Linux files within the same
  workflow run, under one version, each with an attestation.
- **SC-010**: An export from either platform imports on the other with
  0 day whose figures differ.
- **SC-011**: An AppImage one version behind updates itself and restarts in
  under 1 minute on a normal connection, without a terminal.
- **SC-012**: No string containing « Windows » or « Microsoft » is
  reachable in the Linux interface, checked by a test over the string
  table.

## Assumptions

- The owner can test on at least one real Linux machine, or accepts
  virtual machines for Tier 1 (Q8).
- The project keeps the domain `bientot.dev`, which fixes the Flatpak app
  id (Q2).
- Flathub accepts the permissions of `contracts/packaging.md`; if it
  refuses one, the affected row of the parity matrix moves to the Flatpak
  column as « unavailable » and the AppImage keeps the feature.
- GPUI keeps its X11 and Wayland backends at the pinned revision; a
  dependency update that drops one is a breaking change for this spec.
- Technical assumptions A-1 to A-20 are listed in `research.md` with the
  task that verifies each; no lot may rely on one before its task is done.

## Dependencies

- Constitution 1.3.0 and decision D-026 ratified (Lot 0) before Lot 2
  merges (the gate cites the principle) and before Lot 9 starts.
- Spec 004 (weekly schedule) is independent; its region default (T040
  there) and this spec's system language (T126) share one port.
- Spec 003 (plugins) is unchanged; its tests gain a Linux run.
