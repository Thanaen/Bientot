#!/usr/bin/env bash
# Starts a Linux build of Bientôt as a returning user, with the flyout open, on a throwaway data
# folder (so it sends no usage signal), and prints a Markdown table of the time until it is ready
# and its resident memory at rest. The measures are indications, not gates: a hosted runner has no
# GPU and noisy neighbours. The run fails only when the app is not ready within READY_LIMIT_MS
# (default 10000), exits on its own, or writes to standard error.
#
# Usage: scripts/measure-performance.sh <executable> [--screenshot <file.png>]
#
# It needs a display. CI runs it under Xvfb with openbox and Mesa's software Vulkan driver.
# --screenshot captures the screen once the app has settled, with ImageMagick's `import`.
set -euo pipefail

usage() {
  echo "usage: $0 <executable> [--screenshot <file.png>]" >&2
  exit 2
}

[ $# -ge 1 ] || usage
executable=$1
shift
screenshot=
while [ $# -gt 0 ]; do
  case $1 in
    --screenshot)
      [ $# -ge 2 ] || usage
      screenshot=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

ready_limit_ms=${READY_LIMIT_MS:-10000}
settle_seconds=${SETTLE_SECONDS:-5}

work=$(mktemp -d)
pid=
cleanup() {
  if [ -n "$pid" ]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# Saved settings start the day, past the first-run onboarding.
mkdir "$work/data"
printf '{}' > "$work/data/settings.json"

# An exited child stays a zombie until it is waited for, and `kill -0` still finds a zombie.
running() {
  local state
  state=$(awk '/^State:/ { print $2 }' "/proc/$pid/status" 2>/dev/null) || return 1
  [ -n "$state" ] && [ "$state" != Z ]
}

started=$(date +%s%N)
BIENTOT_READY_FILE="$work/ready" "$executable" --open-flyout --data-dir "$work/data" \
  2> "$work/stderr" &
pid=$!

while [ ! -e "$work/ready" ]; do
  elapsed_ms=$((($(date +%s%N) - started) / 1000000))
  if ! running; then
    status=0
    wait "$pid" || status=$?
    pid=
    echo "The app exited with status $status before it was ready. Standard error:" >&2
    cat "$work/stderr" >&2
    exit 1
  fi
  if [ "$elapsed_ms" -gt "$ready_limit_ms" ]; then
    echo "The app was not ready after ${ready_limit_ms} ms. Standard error:" >&2
    cat "$work/stderr" >&2
    exit 1
  fi
  sleep 0.01
done
ready_ms=$((($(date +%s%N) - started) / 1000000))

sleep "$settle_seconds"
if ! running; then
  echo "The app exited within ${settle_seconds} s of being ready. Standard error:" >&2
  cat "$work/stderr" >&2
  exit 1
fi
rss_kib=$(awk '/^VmRSS:/ { print $2 }' "/proc/$pid/status")
private_kib=$(awk '/^Private_Dirty:/ { print $2 }' "/proc/$pid/smaps_rollup")
if [ -n "$screenshot" ]; then
  mkdir -p "$(dirname "$screenshot")"
  import -window root "$screenshot"
fi

kill "$pid"
wait "$pid" 2>/dev/null || true
pid=

if [ -s "$work/stderr" ]; then
  echo "The app wrote to standard error:" >&2
  cat "$work/stderr" >&2
  exit 1
fi

mib() { awk -v kib="$1" 'BEGIN { printf "%.1f", kib / 1024 }'; }
cat <<EOF
| Measure | Measured | Target |
| --- | --- | --- |
| Ready after start | ${ready_ms} ms | < 1000 ms |
| Resident memory after ${settle_seconds} s, flyout open | $(mib "$rss_kib") MiB | < 60 MiB |
| Private memory written after ${settle_seconds} s | $(mib "$private_kib") MiB | |
EOF
