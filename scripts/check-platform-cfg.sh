#!/usr/bin/env bash
# Fails on a platform condition outside the places the constitution allows (Principle VII):
# `cfg`, `cfg!` and `cfg_attr` on `windows`, `unix`, `target_os` or `target_family` belong in
# crates/bientot-windows/, crates/bientot-linux/, crates/bientot-app/src/platform/ and build.rs
# files. Cargo.toml target tables are not Rust and are not read. An attribute that rustfmt wrapped
# is read as one line, its lines trimmed and joined. Anything else must be an entry of
# scripts/platform-cfg-allowlist.txt, which gives how many lines it covers and why; an entry that
# covers more or fewer lines fails too, so a copy cannot slip in under it and the list shrinks
# with the code.
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
        "$work/crates/bientot-app/src/platform"
    printf '# path | line | count | reason\n' > "$work/allowlist.txt"
    printf 'pub fn add() {}\n// cfg(windows) in a comment\n' > "$work/crates/core/src/lib.rs"
    printf '#[cfg(target_os = "linux")]\nmod x;\n' > "$work/crates/bientot-linux/src/lib.rs"
    printf 'pub const X: bool = cfg!(windows);\n' > "$work/crates/bientot-app/src/platform/mod.rs"
    printf 'fn main() { if cfg!(unix) {} }\n' > "$work/crates/core/build.rs"
    expect pass "conditions in the allowed places"

    printf '#[cfg(windows)]\nfn seeded() {}\n' >> "$work/crates/core/src/lib.rs"
    expect fail "a seeded violation"

    printf 'crates/core/src/lib.rs | #[cfg(windows)] | 1 | a reason\n' >> "$work/allowlist.txt"
    expect pass "the violation, allowed with a reason"

    printf '#[cfg(windows)]\nfn copy() {}\n' >> "$work/crates/core/src/lib.rs"
    expect fail "a second copy of an allowed line"

    sed -i 's/| 1 |/| 2 |/' "$work/allowlist.txt"
    expect pass "both copies counted"

    printf '#[cfg_attr(\n    not(unix),\n    allow(dead_code)\n)]\nfn wrapped() {}\n' > "$work/crates/core/src/wrapped.rs"
    expect fail "an attribute rustfmt wrapped"

    printf 'crates/core/src/wrapped.rs | #[cfg_attr(not(unix),allow(dead_code))] | 1 |\n' >> "$work/allowlist.txt"
    expect fail "an entry without a reason"

    sed -i '$d' "$work/allowlist.txt"
    printf 'crates/core/src/wrapped.rs | #[cfg_attr(not(unix),allow(dead_code))] | 1 | a reason\n' >> "$work/allowlist.txt"
    expect pass "the wrapped attribute, allowed as one line"

    printf 'crates/core/src/gone.rs | #[cfg(unix)] | 1 | a reason\n' >> "$work/allowlist.txt"
    expect fail "an entry that matches nothing"

    ((status == 0)) && echo "check-platform-cfg self-test passed."
    exit "$status"
fi

root=${1:-$(dirname "$script")/..}
allowlist=$(realpath "${2:-$root/scripts/platform-cfg-allowlist.txt}")
cd "$root"

trim() {
    local t=$1
    t=${t#"${t%%[![:space:]]*}"}
    printf '%s' "${t%"${t##*[![:space:]]}"}"
}

failed=0
globs=()
lines=()
counts=()
seen=()
while IFS= read -r entry || [[ -n $entry ]]; do
    [[ -z ${entry//[[:space:]]/} || $entry == '#'* ]] && continue
    IFS='|' read -r glob line count reason <<< "$entry"
    glob=$(trim "$glob")
    line=$(trim "$line")
    count=$(trim "${count:-}")
    reason=$(trim "${reason:-}")
    if [[ -z $glob || -z $line || ! $count =~ ^[1-9][0-9]*$ || -z $reason ]]; then
        echo "::error file=$allowlist::Each entry is « path | line | count | reason »: $entry"
        failed=1
        continue
    fi
    globs+=("$glob")
    lines+=("$line")
    counts+=("$count")
    seen+=(0)
done < "$allowlist"

# Prints path:line:text for each platform condition, a wrapped attribute joined from the line of
# its `cfg` until its parentheses close.
# shellcheck disable=SC2016 # an awk program, not shell
scan='
function depth(s,   opened) { opened = gsub(/\(/, "(", s); return opened - gsub(/\)/, ")", s) }
function emit(line, s) {
    if (s ~ /cfg(_attr)?!?[[:space:]]*\((.*[^[:alnum:]_])?(windows|unix|target_os|target_family)([^[:alnum:]_]|$)/)
        print FILENAME ":" line ":" s
}
FNR == 1 { open = 0 }
{
    text = $0
    sub(/^[[:space:]]+/, "", text)
    sub(/[[:space:]]+$/, "", text)
    if (open) {
        joined = joined text
        level += depth(text)
        if (level <= 0) { open = 0; emit(start, joined) }
        next
    }
    if (text ~ /^\/\//) next
    if (text ~ /cfg(_attr)?!?[[:space:]]*\(/) {
        level = depth(text)
        if (level > 0) { open = 1; start = FNR; joined = text; next }
        emit(FNR, text)
    }
}'

while IFS= read -r hit; do
    path=${hit%%:*}
    rest=${hit#*:}
    number=${rest%%:*}
    text=${rest#*:}
    case $path in
        crates/bientot-windows/* | crates/bientot-linux/* | crates/bientot-app/src/platform/* | */build.rs) continue ;;
    esac
    allowed=0
    for i in "${!globs[@]}"; do
        # shellcheck disable=SC2053 # the entry's path is a glob
        if [[ $path == ${globs[$i]} && $text == "${lines[$i]}" ]]; then
            allowed=1
            seen[i]=$((seen[i] + 1))
            break
        fi
    done
    if ((allowed == 0)); then
        echo "::error file=$path,line=$number::A platform condition outside the platform crates and crates/bientot-app/src/platform/ (constitution, Principle VII). Move it behind a port or a constant of the composition root, or list it with its reason in scripts/platform-cfg-allowlist.txt: $text"
        failed=1
    fi
done < <(find crates -name '*.rs' -print0 | sort -z | xargs -0 awk "$scan")

for i in "${!globs[@]}"; do
    if ((seen[i] != counts[i])); then
        echo "::error file=$allowlist::An entry covers ${counts[$i]} line(s) and matches ${seen[$i]}; correct its count, or move the new condition to the platform code: ${globs[$i]} | ${lines[$i]}"
        failed=1
    fi
done

if ((failed == 0)); then
    echo "No platform condition outside the platform code."
fi
exit "$failed"
