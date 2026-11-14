#!/usr/bin/env bash
# Seeds each fuzz target's corpus: the inputs of the bientot-store test fixtures, the manifests
# and packages of the SDK samples, and the hand-written seeds of seeds/<target>/.
set -euo pipefail
cd "$(dirname "$0")"
fixtures=../crates/bientot-store/tests/fixtures
samples=../crates/bientot-plugin-sdk/examples

seed() {
  local target=$1 fixture=$2
  mkdir -p "corpus/$target"
  local count
  count=$(jq 'length' "$fixtures/$fixture")
  for ((i = 0; i < count; i++)); do
    jq -j ".[$i].input" "$fixtures/$fixture" > "corpus/$target/$fixture-$i"
  done
}

seed codec instants.json
seed codec dates.json
seed settings settings-json.json
cp "$fixtures/settings-custom.json" corpus/settings/

for target in seeds/*/; do
  target=$(basename "$target")
  mkdir -p "corpus/$target"
  cp "seeds/$target"/* "corpus/$target/"
done

# Each sample as a manifest, and as an unsigned package around the smallest WebAssembly header.
mkdir -p corpus/manifest corpus/package
for sample in "$samples"/*/; do
  name=$(basename "$sample")
  cp "$sample/plugin.json" "corpus/manifest/$name.json"
  python3 - "$sample" "corpus/package/$name.bientot-plugin" <<'PY'
import pathlib, sys, zipfile
sample, out = pathlib.Path(sys.argv[1]), sys.argv[2]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as package:
    package.write(sample / "plugin.json", "plugin.json")
    package.writestr("plugin.wasm", b"\0asm\1\0\0\0")
    for locale in sorted((sample / "locales").glob("*.json")):
        package.write(locale, f"locales/{locale.name}")
PY
done
