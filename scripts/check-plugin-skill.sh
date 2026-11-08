#!/usr/bin/env bash
# Builds the plugin that the Claude Code skill `write-a-bientot-plugin` teaches, from the code
# blocks of its SKILL.md, against this checkout's SDK, and takes it through every step the
# skill gives: tests with reference files, clippy, the WebAssembly build, pack, sign and check.
# A change to the SDK that the skill does not follow fails here.
#
# A code block becomes a file when the paragraph just before it names that file in backticks,
# like "`plugin.json`:" or "Put the tests at the end of `src/lib.rs`:". A second block for the
# same file is appended to it.
#
#   scripts/check-plugin-skill.sh
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
target=${CARGO_TARGET_DIR:-$root/target}
export CARGO_TARGET_DIR=$target
skill=plugins/bientot/skills/write-a-bientot-plugin/SKILL.md
work="$target/skill-check"
rm -rf "$work"
mkdir -p "$work"

python3 -I - "$skill" "$work/my-plugin" <<'PY'
import os, re, sys

skill, out = sys.argv[1], sys.argv[2]
lines = open(skill, encoding="utf-8").read().split("\n")
written = set()
i = 0
while i < len(lines):
    if not lines[i].startswith("```"):
        i += 1
        continue
    end = i + 1
    while not lines[end].startswith("```"):
        end += 1
    body = "\n".join(lines[i + 1:end]) + "\n"
    # The paragraph before the block.
    j = i - 1
    while j >= 0 and not lines[j].strip():
        j -= 1
    paragraph = []
    while j >= 0 and lines[j].strip():
        paragraph.insert(0, lines[j])
        j -= 1
    text = " ".join(paragraph)
    match = re.match(r"`([\w./-]+\.(?:toml|json|rs))`", text) or re.search(
        r"`([\w./-]+\.(?:toml|json|rs))`:$", text
    )
    if match:
        path = os.path.join(out, match.group(1))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "a" if path in written else "w", encoding="utf-8") as f:
            f.write(body)
        written.add(path)
    i = end + 1

expected = {"Cargo.toml", ".cargo/config.toml", "plugin.json", "src/lib.rs"}
missing = expected - {os.path.relpath(p, out) for p in written}
if missing:
    sys.exit(f"{skill}: no code block for {', '.join(sorted(missing))}")
PY

plugin="$work/my-plugin"
# This checkout's SDK, the workspace's versions, and a workspace of its own.
python3 -I - "$plugin/Cargo.toml" "$root/crates/bientot-plugin-sdk" <<'PY'
import sys

path, sdk = sys.argv[1], sys.argv[2]
manifest = open(path, encoding="utf-8").read()
git = 'bientot-plugin-sdk = { git = "https://github.com/Thanaen/Bientot" }'
if git not in manifest:
    sys.exit(f"Cargo.toml no longer depends on the SDK as « {git} »")
manifest = manifest.replace(git, f'bientot-plugin-sdk = {{ path = "{sdk}" }}')
open(path, "w", encoding="utf-8").write(manifest + "\n[workspace]\n")
PY
cp Cargo.lock "$plugin/"
for tag in $(python3 -I -c 'import json,sys; print(*json.load(open(sys.argv[1]))["locales"])' "$plugin/plugin.json"); do
    [[ -f $plugin/locales/$tag.json ]] || { echo "$skill: no code block for locales/$tag.json" >&2; exit 1; }
done

cargo build --quiet --locked -p bientot-plugin-tool
tool="$target/debug/bientot-plugin"
(
    cd "$plugin"
    BIENTOT_BLESS=1 cargo test --quiet
    cargo test --quiet
    cargo clippy --quiet --target wasm32-unknown-unknown -- -D warnings
    cargo build --quiet --release --target wasm32-unknown-unknown
)
package="$work/my-plugin.bientot-plugin"
"$tool" pack "$plugin" "$target/wasm32-unknown-unknown/release/my_plugin.wasm" --out "$package"
# The key goes in a folder that does not exist yet, as ~/.bientot/ on a first key.
"$tool" keygen "$work/keys/plugin.key"
"$tool" sign "$package" --key "$work/keys/plugin.key"
"$tool" check "$package" --expect "$plugin/expected"
