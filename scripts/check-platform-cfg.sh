#!/usr/bin/env bash
# Fails on a platform condition outside the places the constitution allows (Principle VII):
# `cfg`, `cfg!` and `cfg_attr` on `windows`, `unix`, `target_os` or `target_family` belong in
# crates/bientot-windows/, crates/bientot-linux/, crates/bientot-app/src/platform/ and build.rs
# files. Cargo.toml target tables are not Rust and are not read. Anything else must be a line of
# scripts/platform-cfg-allowlist.txt, which gives its reason; an entry that matches no line fails
# too, so the list shrinks with the code. An attribute split over several lines is not seen.
#
#   scripts/check-platform-cfg.sh [<root> [<allowlist>]]
#   scripts/check-platform-cfg.sh --self-test
set -euo pipefail

script=$(realpath "${BASH_SOURCE[0]}")

if [[ ${1:-} == --self-test ]]; then
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    status=0
    expect() {
        local want=$1 name=$2 got=0
        "$script" "$work" "$work/allowlist.txt" > "$work/out" 2>&1 || got=$?
        if { [[ $want == pass ]] && ((got != 0)); } || { [[ $want == fail ]] && ((got == 0)); }; then
            echo "self-test: $name: expected $want, got exit $got" >&2
            cat "$work/out" >&2
            status=1
        fi
    }
    mkdir -p "$work/crates/core/src" "$work/crates/bientot-linux/src" \
        "$work/crates/bientot-app/src/platform" "$work/crates/core/tests"
    printf '# path | line | reason\n' > "$work/allowlist.txt"
    printf 'pub fn add() {}\n// cfg(windows) in a comment\n' > "$work/crates/core/src/lib.rs"
    printf '#[cfg(target_os = "linux")]\nmod x;\n' > "$work/crates/bientot-linux/src/lib.rs"
    printf 'pub const X: bool = cfg!(windows);\n' > "$work/crates/bientot-app/src/platform/mod.rs"
    printf 'fn main() { if cfg!(unix) {} }\n' > "$work/crates/core/build.rs"
    expect pass "conditions in the allowed places"

    printf '#[cfg(windows)]\nfn seeded() {}\n' >> "$work/crates/core/src/lib.rs"
    expect fail "a seeded violation"

    printf 'crates/core/src/lib.rs | #[cfg(windows)] | a reason\n' >> "$work/allowlist.txt"
    expect pass "the violation, allowed with a reason"

    printf '    #[cfg(windows)]\n' > "$work/crates/core/tests/a.rs"
    expect fail "the same line in another file"

    printf 'crates/core/tests/*.rs | #[cfg(windows)] |\n' >> "$work/allowlist.txt"
    expect fail "an entry without a reason"

    printf 'crates/core/tests/*.rs | #[cfg(windows)] | a reason\n' > "$work/entry"
    sed -i '$d' "$work/allowlist.txt"
    cat "$work/entry" >> "$work/allowlist.txt"
    expect pass "a glob entry"

    printf 'crates/core/src/gone.rs | #[cfg(unix)] | a reason\n' >> "$work/allowlist.txt"
    expect fail "an entry that matches nothing"

    ((status == 0)) && echo "check-platform-cfg self-test passed."
    exit "$status"
fi

root=${1:-$(dirname "$script")/..}
allowlist=$(realpath "${2:-$root/scripts/platform-cfg-allowlist.txt}")
cd "$root"

failed=0
globs=()
lines=()
reasons=()
used=()
while IFS= read -r entry || [[ -n $entry ]]; do
    [[ -z ${entry//[[:space:]]/} || $entry == '#'* ]] && continue
    IFS='|' read -r glob line reason <<< "$entry"
    trim() { local t=$1; t=${t#"${t%%[![:space:]]*}"}; printf '%s' "${t%"${t##*[![:space:]]}"}"; }
    glob=$(trim "$glob")
    line=$(trim "$line")
    reason=$(trim "${reason:-}")
    if [[ -z $glob || -z $line || -z $reason ]]; then
        echo "::error file=$allowlist::Each entry is « path | line | reason »: $entry"
        failed=1
        continue
    fi
    globs+=("$glob")
    lines+=("$line")
    reasons+=("$reason")
    used+=(0)
done < "$allowlist"

condition='cfg(_attr)?!?[[:space:]]*\(.*\b(windows|unix|target_os|target_family)\b'
while IFS= read -r hit; do
    path=${hit%%:*}
    rest=${hit#*:}
    number=${rest%%:*}
    text=${rest#*:}
    text=${text#"${text%%[![:space:]]*}"}
    text=${text%"${text##*[![:space:]]}"}
    [[ $text == //* ]] && continue
    case $path in
        crates/bientot-windows/* | crates/bientot-linux/* | crates/bientot-app/src/platform/* | */build.rs) continue ;;
    esac
    allowed=0
    for i in "${!globs[@]}"; do
        # shellcheck disable=SC2053 # the entry's path is a glob
        if [[ $path == ${globs[$i]} && $text == "${lines[$i]}" ]]; then
            allowed=1
            used[i]=1
        fi
    done
    if ((allowed == 0)); then
        echo "::error file=$path,line=$number::A platform condition outside the platform crates and crates/bientot-app/src/platform/ (constitution, Principle VII). Move it behind a port or a constant of the composition root, or list it with its reason in scripts/platform-cfg-allowlist.txt: $text"
        failed=1
    fi
done < <(grep -rnE --include='*.rs' "$condition" crates | sort)

for i in "${!globs[@]}"; do
    if ((used[i] == 0)); then
        echo "::error file=$allowlist::An entry matches no line; remove it: ${globs[$i]} | ${lines[$i]}"
        failed=1
    fi
done

if ((failed == 0)); then
    echo "No platform condition outside the platform code."
fi
exit "$failed"
