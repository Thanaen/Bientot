# Contract: packaging, release files and CI gates

Reasons are in `research.md`, R13 and R15.

## Identity

| Item | Value |
| --- | --- |
| App id | `dev.bientot.Bientot` (Q2; cannot change after Flathub publication) |
| Desktop entry | `dev.bientot.Bientot.desktop`, `Name=Bientôt`, `Categories=Office;Utility;`, `StartupWMClass=dev.bientot.Bientot`, `Exec=bientot` |
| Metainfo | `dev.bientot.Bientot.metainfo.xml`, summary and description in `en` and `fr`, screenshots at stable URLs under `bientot.dev`, `<releases>` generated from the release summaries, OARS content rating, `<url type="vcs-browser">` |
| Icons | `hicolor/{64x64,128x128,256x256}/apps/dev.bientot.Bientot.png`, `hicolor/scalable/apps/dev.bientot.Bientot.svg` |
| Executable name | `bientot` on Linux (Q11). Windows keeps `worktime.exe`, which installed copies and the Store package depend on |

Sources live in `packaging/linux/`.

## Flatpak (channel `flatpak`)

| Item | Value |
| --- | --- |
| Manifest | `packaging/linux/dev.bientot.Bientot.yml`, mirrored to the Flathub repository by workflow |
| Runtime | `org.freedesktop.Platform` and `Sdk`, current stable; `org.freedesktop.Sdk.Extension.rust-stable` |
| Build | `cargo --offline build --release --locked -p bientot-app --bin bientot --features release`; sources `cargo-sources.json` generated from `Cargo.lock` |
| Features | `release` (no updater), as the Store build |
| Architectures | x86_64, aarch64 (Flathub builds both) |
| Cadence | Weekly with the Store submission, same commit; manual run for urgent fixes (Q13) |

`finish-args`, complete list, each with its reason for the Flathub review:

| Permission | Reason |
| --- | --- |
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc` | Display |
| `--device=dri` | GPU rendering |
| `--share=network` | Plugin revocation list; usage signal (opt-out) |
| `--talk-name=org.kde.StatusNotifierWatcher` | Tray icon |
| `--talk-name=org.gnome.Mutter.IdleMonitor`, `--talk-name=org.cinnamon.Muffin.IdleMonitor`, `--talk-name=org.freedesktop.ScreenSaver`, `--talk-name=org.gnome.ScreenSaver` | Idle time and lock state, the app's purpose |
| `--system-talk-name=org.freedesktop.login1` | Lock, sleep and resume events |

No `--filesystem`, no `--socket=session-bus`, no `--device=all`.

## AppImage (channel `appimage`)

| Item | Value |
| --- | --- |
| Built by | `vpk [linux] pack` (Velopack 1.2.161, packId `Thanaen.WorkTime` as on Windows) from `scripts/release-direct.sh --target linux-x64` or `linux-arm64`, on a host of that architecture; needs `mksquashfs` |
| Channels | `linux` (x86_64, Velopack's default) and `linux-arm64`: on the default channel both architectures would write `releases.linux.json`. An installed copy reads the channel it was packed with from `usr/bin/sq.version` (velopack's `get_practical_channel`), so the app names none |
| Features | `direct` (Velopack updater) |
| Updates | As `Setup.exe` (`platform/direct.rs`), only when the app runs as an AppImage (`APPIMAGE` set, not a Flatpak): a daily check of `releases.<channel>.json` under the feed, the package downloaded to `/var/tmp/velopack/Thanaen.WorkTime/packages`, installed at the next start or with « Redémarrer maintenant »; `UpdateNix` replaces the file `$APPIMAGE` names and starts it again (T143) |
| Build host | `ubuntu-22.04` and `ubuntu-22.04-arm` runners, or a `debian:12` container (A-19). CI: `Rust on Linux` and `Rust on Linux (aarch64)` |
| Contents | `AppRun` (a shell script that `exec`s the `Exec=` of the desktop file), `Thanaen.WorkTime.desktop` (`Name=Bientôt`, `Exec=worktime`, `Icon=Thanaen.WorkTime`, `StartupWMClass=Thanaen.WorkTime`, `Categories=Office;Utility;`), the icon as `.DirIcon` and `Thanaen.WorkTime.png` (256 × 256), and `usr/bin/`: `worktime`, Velopack's static `UpdateNix`, `sq.version`, the license files and `THIRD-PARTY-NOTICES.txt`. `AppRun` also copies the icon into `~/.cache/thumbnails/` |
| Runtime | AppImage type 2, static; mounts through FUSE (`fusermount3` where tried). Without a `fusermount` it says so on standard output, and the app still starts |
| glibc | Highest `GLIBC_` symbol version ≤ 2.35, checked with `objdump -T`. Measured 27 November 2026: 2.35 (`hypotf` only), 2.34 for every other symbol |
| Dynamic libraries allowed | `libc`, `libm`, `libgcc_s`, `libdl`, `libpthread`, `librt`, and those GPUI loads at run time (`libwayland-client`, `libxkbcommon`, `libxcb*`, `libvulkan`); **not** `libssl`, `libcrypto`. Linked on 27 November 2026 (`DT_NEEDED`): `libxkbcommon`, `libxkbcommon-x11`, `libxcb`, `libgcc_s`, `libm`, `libc` |
| Self-integration | On first run, writes `~/.local/share/applications/dev.bientot.Bientot.desktop` pointing at `$APPIMAGE` and the icons under `~/.local/share/icons/hicolor/`; « Remove from the app menu » in the settings deletes them. The AppImage's own desktop file keeps vpk's names |

## Release files

One GitHub release per version carries, beside today's Windows files:

| File | What |
| --- | --- |
| `Bientot-x86_64.AppImage`, `Bientot-aarch64.AppImage` | Downloads: copies of vpk's `Thanaen.WorkTime.AppImage` and `Thanaen.WorkTime-linux-arm64.AppImage`, as `Bientot-Setup.exe` is of `Thanaen.WorkTime-win-Setup.exe` |
| `releases.linux.json`, `releases.linux-arm64.json` | Velopack feeds |
| `assets.linux.json`, `RELEASES-linux`, and the `linux-arm64` pair | Also written by vpk |
| `Thanaen.WorkTime-<version>-linux-full.nupkg`, `…-linux-delta.nupkg`, and the `linux-arm64` pair | Update packages; the feed names them, so they keep vpk's names |
| `Bientot.intoto.jsonl` | Gains the attestations of every file above |

The file and channel names are those of a real `vpk pack` run on
27 November 2026 (T142). Task T140 checks that an installed AppImage finds
and applies them.

Service routes (`services/bientot/src/releases.ts`):

| Route | Answer |
| --- | --- |
| `/download` | By `User-Agent`: Linux → the site's Linux section; otherwise `Bientot-Setup.exe` as today |
| `/download/windows` | `Bientot-Setup.exe` |
| `/download/linux`, `/download/linux-arm64` | The AppImage of that architecture |
| `/updates/<file>` | Also serves the Linux feeds and packages |

## CI gates

| Gate | Job | Fails when |
| --- | --- | --- |
| Lint and tests | `Rust` | as today |
| Windows cross-lint | `Rust` | `cargo clippy --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings` fails |
| Platform conditions | `Rust` | `scripts/check-platform-cfg.sh` finds one outside the allowed places |
| Size | `Rust on Linux` | `stat -c %s` of the stripped `bientot` (`release`, then `direct`) > 20 000 000 |
| glibc | `Rust on Linux` | highest `GLIBC_` version > 2.35, in the release executable or in `worktime` or `UpdateNix` inside the AppImage |
| No OpenSSL | `Rust on Linux` | `ldd` lists `libssl` or `libcrypto` |
| Headless start | `Rust on Linux` | no ready file within 10 s under `Xvfb` and under `sway` headless, or anything on standard error; under `Xvfb`, the release executable and the AppImage |
| AppImage build | `Rust on Linux` | `scripts/release-direct.sh --target linux-x64` fails |
| Desktop harness | `Rust on Linux` | an integration test of tray, placement, idle, logind, notifications, autostart or single instance fails |
| Hostile plugin corpus | `Rust on Linux` | as on Windows |
| Flatpak build | `Flatpak` (on changes to `packaging/linux/` or `Cargo.lock`, and weekly) | `flatpak-builder` fails, `flatpak-builder-lint` reports an error, or `cargo-sources.json` is stale |
| Release | `release.yml` | any platform's build fails (Q6), or a file lacks its attestation |

Reported, not enforced: start time, resident memory, AppImage file size,
aarch64 executable size (gated at 20 MB in the release workflow only).
`Rust on Linux (aarch64)` runs the gates of `Rust on Linux` on Arm without
being a required check. `Linux screenshot` starts what `Rust on Linux` built
on Ubuntu 24.04, whose Mesa draws the flyout under Xvfb, and keeps a
screenshot.

## Release summary files

```text
platforms: linux          # optional first line: windows | linux
fr: Bientôt …
en: Bientôt …
```

Without the first line the entry concerns both. `release-notes.sh` prints
common lines first, then « Windows » and « Linux » sections when either has
lines. `store-release-notes.sh` drops `platforms: linux` entries; the
metainfo generator drops `platforms: windows` entries.
