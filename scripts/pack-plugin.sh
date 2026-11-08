#!/usr/bin/env bash
# Packs a plugin folder (plugin.json, locales/, assets/) and its WebAssembly module into a
# .bientot-plugin file, after the checks Bientôt makes at install: the manifest, the locale files,
# and every import of the module against the declared capabilities.
#
#   scripts/pack-plugin.sh <plugin folder> <module.wasm> [--out <file.bientot-plugin>]
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run --quiet --locked -p bientot-plugin-tool -- pack "$@"
