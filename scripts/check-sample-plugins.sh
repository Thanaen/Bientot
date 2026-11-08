#!/usr/bin/env bash
# Builds every sample plugin of the SDK for wasm32-unknown-unknown, packs it, and runs it in
# Bientôt's own runtime on the demo days: its cards must match the reference files its native
# tests check, so the WebAssembly build behaves as the tests say. The packages land in
# target/plugins/.
#
#   scripts/check-sample-plugins.sh
set -euo pipefail
cd "$(dirname "$0")/.."
target=${CARGO_TARGET_DIR:-target}
mkdir -p "$target/plugins"
cargo build --quiet --locked -p bientot-plugin-tool
tool="$target/debug/bientot-plugin"
for dir in crates/bientot-plugin-sdk/examples/*/; do
    dir=${dir%/}
    name=$(basename "$dir")
    cargo build --quiet --locked -p bientot-plugin-sdk --example "$name" --release \
        --target wasm32-unknown-unknown
    package="$target/plugins/$name.bientot-plugin"
    "$tool" pack "$dir" "$target/wasm32-unknown-unknown/release/examples/$name.wasm" --out "$package"
    "$tool" check "$package" --expect "$dir/expected"
done
