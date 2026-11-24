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
| Built by | `vpk pack` (Velopack 1.2.161) from `scripts/release-direct.sh --target linux-x64` or `linux-arm64` |
| Features | `direct` (Velopack updater) |
| Build host | `ubuntu-22.04` and `ubuntu-22.04-arm` runners, or a `debian:12` container (A-19) |
| glibc | Highest `GLIBC_` symbol version ≤ 2.35, checked with `objdump -T` |
| Dynamic libraries allowed | `libc`, `libm`, `libgcc_s`, `libdl`, `libpthread`, `librt`, and those GPUI loads at run time (`libwayland-client`, `libxkbcommon`, `libxcb*`, `libvulkan`); **not** `libssl`, `libcrypto` |
| Self-integration | On first run, writes `~/.local/share/applications/dev.bientot.Bientot.desktop` pointing at `$APPIMAGE` and the icons under `~/.local/share/icons/hicolor/`; « Remove from the app menu » in the settings deletes them |

## Release files

One GitHub release per version carries, beside today's Windows files:

| File | What |
| --- | --- |
| `Bientot-x86_64.AppImage`, `Bientot-aarch64.AppImage` | Downloads |
| `releases.linux.json`, `releases.linux-arm64.json` | Velopack feeds |
| `Bientot-<version>-linux-full.nupkg`, `…-linux-delta.nupkg`, and the `linux-arm64` pair | Update packages |
| `Bientot.intoto.jsonl` | Gains the attestations of every file above |

Exact Velopack file and channel names are fixed by task T140 from a real
`vpk pack` run and corrected here in the same pull request.

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
| glibc | `Rust on Linux` | highest `GLIBC_` version > 2.35 |
| No OpenSSL | `Rust on Linux` | `ldd` lists `libssl` or `libcrypto` |
| Headless start | `Rust on Linux` | no ready file within 10 s under `Xvfb` and under `sway` headless, or anything on standard error |
| Desktop harness | `Rust on Linux` | an integration test of tray, placement, idle, logind, notifications, autostart or single instance fails |
| Hostile plugin corpus | `Rust on Linux` | as on Windows |
| Flatpak build | `Flatpak` (on changes to `packaging/linux/` or `Cargo.lock`, and weekly) | `flatpak-builder` fails, `flatpak-builder-lint` reports an error, or `cargo-sources.json` is stale |
| Release | `release.yml` | any platform's build fails (Q6), or a file lacks its attestation |

Reported, not enforced: start time, resident memory, AppImage file size,
aarch64 executable size (gated at 20 MB in the release workflow only).

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
