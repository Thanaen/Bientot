# Platforms

Bientôt is built for Windows and Linux as equals (constitution, Principle VII).
This page lists what each platform supports and where they differ. It changes
by pull request, together with the code it describes.

Linux support is in progress: the Linux packages are not published yet. The
plan is `specs/005-linux-support/`.

## Windows

Windows 11 (build 22000 or later), on x64. Windows 10 and Arm are not
supported (D-013 in `docs/product-decisions.md`). Bientôt ships through the
Microsoft Store (weekly) and a self-updating installer (nightly).

## Linux

Bientôt ships on Flathub (`dev.bientot.Bientot`, weekly, built by Flathub from
the tagged source) and as an AppImage that updates itself (nightly), for
x86_64 and aarch64. The project maintains no `.deb`, `.rpm`, Snap or AUR
package; community packages are welcome, and the simplest wraps the AppImage.

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

Tier 1 desktops are checked in virtual machines before a release that touches
the Linux shell. Suspend and two monitors are simulated there.

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
| Usage signal | 3 fields | Same fields, channel `appimage`, plus `desktop` and `session` (Q3) | Same | Same | Channel `flatpak` |
| Demo mode, `--data-dir`, safe mode | Yes | Same | Same | Same | |
| Architectures | x64 | x86_64, aarch64 | Same | Same | Same |

On a compositor that offers only version 1 of the idle protocol, an app that
inhibits idle (a video call) makes the user look active. Bientôt accepts that
and writes it to the diagnostic log (D-026).
