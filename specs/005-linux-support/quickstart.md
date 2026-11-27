# Quickstart: validating Linux support

Two parts: what a developer runs on any Linux machine, and the manual pass
on a real desktop that CI cannot replace.

## Build and run from source

```sh
# Ubuntu 24.04; other distributions: the same libraries under their names
sudo apt-get install -y build-essential pkg-config libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libvulkan1 mesa-vulkan-drivers

cargo test --workspace --locked
cargo run -p bientot-app --bin bientot -- --data-dir /tmp/bientot-try --open-flyout

# what CI runs
scripts/check-platform-cfg.sh
scripts/linux-desktop-harness.sh x11      # Xvfb + openbox + D-Bus mocks
scripts/linux-desktop-harness.sh wayland  # sway headless + D-Bus mocks
scripts/measure-performance.sh target/release/bientot
```

The package list is indicative: the `Rust` CI job installs nothing today
(the runner image already holds the libraries), and the screenshots job
adds `mesa-vulkan-drivers`. Task T030 fixes the exact list in
`CONTRIBUTING.md`. The executable is called `worktime` until Q11 is
answered and task T114 renames the Linux one to `bientot`.

Checking Windows from Linux (no Windows machine needed):

```sh
cargo clippy --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings
```

with the cross tools of `CONTRIBUTING.md`.

## Manual pass on a real desktop

Run on each Tier 1 desktop (`spec.md`) before the launch and before any
release that touches `bientot-linux` or `platform/linux.rs`. Use a fresh
user account or `--data-dir`. Record date, distribution, desktop version,
session type, install kind and the result of each step in `research.md`,
« Validation results ».

**Install and first start**

1. Install the package under test (Flatpak or AppImage). Bientôt appears
   in the app grid with its name and icon.
2. Start it. Onboarding shows; no text names Windows.
3. Finish onboarding. On a desktop with a tray host, the icon shows in the
   panel with the remaining time and is legible on this panel's colour.
4. On Fedora GNOME (no host): a window opens instead, with the notice;
   « Comment afficher l’icône » opens the help page. Install the AppIndicator
   extension: the icon appears and the window becomes the flyout, without
   a restart.

**Tray and flyout**

5. Primary click on the icon: the flyout opens in under a second, at the
   place the parity matrix gives for this session.
6. Esc closes it. Open again; click on the desktop: it closes. Open again;
   click the icon: it closes.
7. Secondary click: the menu shows the same entries as the Windows
   reference screenshot; « Ouvrir Bientôt » opens the flyout; « Quitter »
   quits.
8. With two monitors: click the icon on each (when the panel is on both);
   the flyout opens on the clicked one (X11), or on the focused output
   (Wayland).
9. Rounded corners and shadow match the mockup; nothing is clipped at the
   desktop's scale (try 100 %, 125 %, 200 %).
10. Tab through the flyout; every control is reachable and shows focus.

**Activity**

11. Open Settings › Diagnostic: the idle backend named is the one
    `contracts/linux-backends.md` gives for this session.
12. Set the idle threshold to its minimum. Work for 3 minutes, leave the
    keyboard and pointer for threshold + 1 minute, return. The day shows
    the idle span with the right bounds (± 10 s).
13. Play a video full screen for threshold + 1 minute without touching
    anything. The span is idle (the player's inhibitor is ignored). On a
    compositor with protocol version 1 only, note the difference.

**Lock, sleep, session**

14. Lock the screen for 2 minutes, unlock. The lock shows in the day with
    the right bounds.
15. Suspend for 2 minutes, resume. The sleep shows in the day.
16. Switch to another user and back. The gap is treated as a lock.
17. Sign out with Bientôt running, sign in: Bientôt starts hidden (if the
    startup switch is on) and the day continues without a duplicate or a
    hole other than the sign-out itself.

**History**

18. Quit Bientôt, suspend the computer for 2 minutes, resume, start
    Bientôt. With a readable journal the sleep is recorded with its real
    bounds; in the Flatpak the time is unknown and offered for
    declaration.
19. On a new data folder at mid-day: « Avant Bientôt » offers the time
    since the session opened, with the Linux text; accepting and refusing
    both work.

**Everyday integration**

20. Trigger a reminder (demo mode, or set the rule a minute ahead): a
    desktop notification shows with the right text. Click it: the flyout
    opens.
21. Turn the desktop's « Do not disturb » on, trigger again: nothing
    shows and Bientôt reports nothing wrong.
22. Settings › Startup: turn on, sign out and in, Bientôt starts hidden.
    Turn off, repeat, it does not start. (Flatpak: the desktop may ask
    once.)
23. AppImage only: move the file, start it, open the settings: the
    startup state says the registration points elsewhere; « Corriger »
    repairs it.
24. Launch Bientôt again from the app grid while it runs: the running one
    opens its flyout; `pgrep -c bientot` stays 1.
25. Switch the desktop between light and dark: Bientôt follows. Turn
    animations off in the desktop's accessibility settings: Bientôt's
    transitions stop.
26. Start Orca: the flyout's remaining time, the buttons and the week are
    read with the names Narrator reads.

**Data**

27. Export a `.bientot` file; import one made on Windows. Every past day
    shows the Windows figures.
28. Install a plugin from a file and by dropping it on the settings page;
    it runs. Start with `--no-plugins`: it does not.
29. Export diagnostics: the archive names the distribution, desktop,
    session type, install kind and backends, and nothing personal beyond
    the Windows export.

**Update**

30. AppImage: install the previous version, publish or point to a newer
    feed, accept the update: the file is replaced and Bientôt restarts on
    the new version. Flatpak: `flatpak update` brings the new version and
    the settings show no update section.

## Windows pass (after Lots 1 and 2, and before the launch)

The existing checks, unchanged: `scripts/verify-package-*.ps1`,
`verify-startup-visibility.ps1`, `test-rust-toast-activation.ps1`, then by
hand: tray icon on light and dark taskbar, flyout on each of two monitors
at different scales, toast click with Bientôt closed, lock and sleep in
the day, Store or installer update prompt.
