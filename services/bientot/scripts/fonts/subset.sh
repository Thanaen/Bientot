#!/bin/sh
# Rebuilds the site's font subsets (site/fonts), their @font-face rules and preloads in the pages,
# and src/fonts.ts. Run it by hand from services/bientot after changing the pages' text or styles,
# then commit what it writes. Needs bun, Playwright's Chromium, python3 and fonttools with brotli
# (pip install fonttools brotli). Extra arguments go to subset.py (--no-hinting).
set -e
cd "$(dirname "$0")/../.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
bun scripts/fonts/pages.ts "$tmp/pages" > /dev/null
bun scripts/fonts/browse.ts "$tmp/pages" . "$tmp/usage.json" > /dev/null
python3 scripts/fonts/subset.py "$tmp/usage.json" "$@"
