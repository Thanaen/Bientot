#!/usr/bin/env bash
# Builds the Velopack release of Bientôt for one platform, the full and delta packages and the
# feed that installed copies read:
#
#   win-x64      `Setup.exe` and `releases.win.json`
#                (crates/bientot-app/src/platform/direct.rs), with the sample plugin
#                `focus.bientot-plugin`. The Release workflow (.github/workflows/release.yml) runs
#                it on Windows and publishes the output folder as a GitHub Release; it also
#                cross-builds from Linux.
#   linux-x64    The AppImage and `releases.linux.json`, built on a Linux host of the same
#   linux-arm64  architecture, whose glibc is the oldest the AppImage runs on: an Ubuntu 22.04
#                runner, glibc 2.35 (specs/005-linux-support/contracts/packaging.md). The arm64
#                feed is `releases.linux-arm64.json`: on the default channel both architectures
#                would write `releases.linux.json`. CI builds both on every Rust pull request.
#
#   scripts/release-direct.sh --target {win-x64|linux-x64|linux-arm64} <output-dir>
#
# Releases come from a clean, complete checkout of main, so that a version always names the same
# bits: RELEASE_ANY_BRANCH=1 lifts the branch check for a test into another feed (RELEASE_FEED).
#
# Needs jq, Pillow (PYTHON names the interpreter), cargo-xwin off Windows, and vpk of the velopack crate's version, whose Update.exe installs
# the updates (`dotnet tool install vpk --version 1.2.161`; VPK names it when not on PATH). On
# Linux, vpk also needs mksquashfs (squashfs-tools).
# gpui-pre-windows compiles its shaders only on a Windows host: from Linux, when its
# shaders_bytes.rs is missing, set FXC to a fxc.exe that runs under Wine and GEN_SHADERS to the script that writes
# the file from it (see the worktime-windows-cross-build note).
set -euo pipefail

usage() {
    echo "usage: release-direct.sh --target {win-x64|linux-x64|linux-arm64} <output-dir>" >&2
    exit 2
}
platform=
output=
while [[ $# -gt 0 ]]; do
    case $1 in
        --target)
            [[ $# -ge 2 ]] || usage
            platform=$2
            shift 2
            ;;
        -*) usage ;;
        *)
            [[ -z $output ]] || usage
            output=$1
            shift
            ;;
    esac
done
[[ -n $output ]] || usage
root=$(cd "$(dirname "$0")/.." && pwd)
feed=${RELEASE_FEED:-https://worktime.thanaen.dev/updates/}
vpk=${VPK:-vpk}
python=${PYTHON:-python3}
target_dir=${CARGO_TARGET_DIR:-$root/target}

fail() {
    echo "release-direct.sh: $*" >&2
    exit 1
}
# The Velopack channel and the Rust target. An installed copy updates from the channel it was
# packed with, which its package records.
case $platform in
    win-x64) channel=win target=x86_64-pc-windows-msvc ;;
    linux-x64) channel=linux target=x86_64-unknown-linux-gnu ;;
    linux-arm64) channel=linux-arm64 target=aarch64-unknown-linux-gnu ;;
    *) usage ;;
esac
# A Linux build is native: the host's glibc is the oldest the AppImage runs on, and GPUI links the
# host's X11 libraries.
if [[ $platform == linux-* && $(uname -sm) != "Linux ${target%%-*}" ]]; then
    fail "$platform builds on a Linux ${target%%-*} host"
fi
[[ -z $(git -C "$root" status --porcelain) ]] || fail "the checkout has uncommitted or untracked files"
[[ $(git -C "$root" rev-parse --is-shallow-repository) == false ]] || fail "the clone is shallow: its commit count is not main's"
if [[ ${RELEASE_ANY_BRANCH:-} != 1 ]]; then
    # RELEASE_NO_FETCH=1: the caller set origin/main, as the Release workflow does.
    [[ ${RELEASE_NO_FETCH:-} == 1 ]] || git -C "$root" fetch --quiet origin main
    [[ $(git -C "$root" rev-parse HEAD) == $(git -C "$root" rev-parse origin/main) ]] ||
        fail "releases come from origin/main as pushed (RELEASE_ANY_BRANCH=1 for a test feed)"
fi
[[ ! -e $output || -z $(ls -A "$output") ]] || fail "$output is not empty: its files would be published with the release"

# Every commit of main gets a higher version than the one before.
# tr: jq.exe ends its lines with CRLF under Git Bash on Windows.
base=$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" |
    jq -r '.packages[] | select(.name == "bientot-app") | .version' | tr -d '\r')
# <major>.<minor>.<commit count>, as the Store package's first three numbers.
IFS=. read -r major minor _ <<< "$base"
version="$major.$minor.$(git -C "$root" rev-list --count HEAD)"
# The version the usage ping accepts (services/bientot/src/ping.ts), which the app parses.
[[ $version =~ ^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,5}(-[0-9A-Za-z.]{1,20})?$ ]] || fail "unexpected version $version"
echo "Bientôt $version"

export XWIN_ACCEPT_LICENSE=1 CARGO_TARGET_DIR=$target_dir
# No VCRUNTIME140.dll to install next to the exe.
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static"
# « Publiée le » in Paramètres: the day of the commit, so that a version always names the same day.
released=$(git -C "$root" log -1 --format=%cs HEAD)
[[ $released =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || fail "unexpected commit date $released"
export BIENTOT_CHANNEL=direct BIENTOT_VERSION=$version BIENTOT_RELEASE_DATE=$released
cargo_build=(cargo xwin build)
[[ $OSTYPE == msys || $OSTYPE == cygwin ]] && cargo_build=(cargo build)
build() {
    "${cargo_build[@]}" --release --locked -p bientot-app --bin worktime --features direct \
        --target "$target" --manifest-path "$root/Cargo.toml"
}
if [[ $platform == linux-* ]]; then
    # The host's target, in target/release, where CI's other Linux builds share the dependencies.
    cargo build --release --locked -p bientot-app --bin worktime --features direct \
        --manifest-path "$root/Cargo.toml"
elif ! build; then
    compgen -G "$target_dir/$target/release/build/gpui-pre-windows-*/out" >/dev/null ||
        fail "the build failed before gpui-pre-windows' build script ran"
    out=$(ls -dt "$target_dir/$target/release/build/gpui-pre-windows-"*/out | head -n 1)
    [[ ! -f "$out/shaders_bytes.rs" ]] || fail "the build failed, and not on the shaders"
    [[ -n ${FXC:-} && -n ${GEN_SHADERS:-} ]] ||
        fail "the build needs gpui-pre-windows' shaders: set FXC and GEN_SHADERS"
    manifest=$(cargo metadata --format-version 1 --locked --manifest-path "$root/Cargo.toml" |
        jq -r '.packages[] | select(.name == "gpui-pre-windows") | .manifest_path' | tr -d '\r')
    "$python" "$GEN_SHADERS" "$(dirname "$manifest")/src" "$out" "$FXC"
    build
fi

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
pack="$staging/pack"
mkdir -p "$pack"
if [[ $platform == win-x64 ]]; then
    cp "$target_dir/$target/release/worktime.exe" "$pack/"
    # PORTABLE_ICON in crates/bientot-windows/src/notifications/native.rs: the toasts' icon.
    cp "$root/packaging/msix/worktime.png" "$pack/"
else
    cp "$target_dir/release/worktime" "$pack/"
fi
cp "$root/LICENSE" "$pack/LICENSE.txt"
cp "$root/crates/bientot-app/assets/icons/LICENSE" "$pack/LICENSE-icons.txt"
cp "$root/crates/bientot-app/assets/fonts/OFL.txt" "$pack/LICENSE-fonts.txt"
if [[ $platform == win-x64 ]]; then
    (cd "$root" && cargo run --locked --quiet -p bientot-notices -- "$pack/THIRD-PARTY-NOTICES.txt" direct)
    "$python" -c 'import sys; from PIL import Image; Image.open(sys.argv[1]).save(sys.argv[2], sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])' \
        "$root/packaging/msix/worktime.png" "$staging/worktime.ico"
else
    (cd "$root" && cargo run --locked --quiet -p bientot-notices -- "$pack/THIRD-PARTY-NOTICES.txt" direct "$target")
    # The AppImage's icon, at the largest size of the hicolor theme the launcher entry uses.
    "$python" -c 'import sys; from PIL import Image; Image.open(sys.argv[1]).resize((256, 256), Image.LANCZOS).save(sys.argv[2])' \
        "$root/packaging/msix/Images/Square150x150Logo.scale-200.png" "$staging/worktime.png"
fi

mkdir -p "$output"
# The previous release, from which vpk computes the delta package. Only the first release,
# FIRST_RELEASE=1, has none: a release never goes out without its delta by mistake.
if [[ ${FIRST_RELEASE:-} == 1 ]]; then
    echo "first release: no previous one at $feed"
else
    # Without the channel, vpk reads the feed of the host OS (releases.linux.json).
    "$vpk" download http --channel "$channel" --url "$feed" --outputDir "$output" ||
        fail "no previous release at $feed (FIRST_RELEASE=1 for the first one)"
fi
if [[ $platform == win-x64 ]]; then
    # The title names the Start menu shortcut and the Apps entry: an update renames both when it
    # changes (Velopack's shortcuts.rs and registry.rs). The id and the exe stay: a new id installs a
    # second copy, and a new exe name leaves the Run value on a missing file.
    "$vpk" "[win]" pack --runtime win-x64 \
        --packId Thanaen.WorkTime --packVersion "$version" --packTitle "Bientôt" --packAuthors Thanaen \
        --packDir "$pack" --mainExe worktime.exe --icon "$staging/worktime.ico" \
        --shortcuts StartMenuRoot --outputDir "$output"
else
    # The same id as on Windows: the channel keeps the two platforms' feeds and packages apart.
    "$vpk" "[linux]" pack --runtime "$platform" --channel "$channel" \
        --packId Thanaen.WorkTime --packVersion "$version" --packTitle "Bientôt" --packAuthors Thanaen \
        --packDir "$pack" --mainExe worktime --icon "$staging/worktime.png" \
        --categories "Office;Utility" --outputDir "$output"
fi
# The sample plugin, which the Store's notes for certification (packaging/store/
# certification-notes.txt) point testers to, and an example for anyone writing one. It runs on
# both platforms, so the Windows build alone packs it.
if [[ $platform == win-x64 ]]; then
    cargo build --release --locked -p bientot-plugin-sdk --example focus --target wasm32-unknown-unknown \
        --manifest-path "$root/Cargo.toml"
    "$root/scripts/pack-plugin.sh" "$root/crates/bientot-plugin-sdk/examples/focus" \
        "$target_dir/wasm32-unknown-unknown/release/examples/focus.wasm" --out "$output/focus.bientot-plugin"
fi
ls -l "$output"
