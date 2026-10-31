#!/usr/bin/env bash
# Seeds each fuzz target's corpus with the inputs of the bientot-store test fixtures.
set -euo pipefail
cd "$(dirname "$0")"
fixtures=../crates/bientot-store/tests/fixtures

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
