#!/usr/bin/env bash
# Runs a command in a throwaway Linux desktop session, as a hosted runner can offer one: a private
# D-Bus session bus and system bus, python-dbusmock's logind and notification server, a
# StatusNotifierWatcher and Mutter's idle monitor (crates/bientot-linux/tests/support/), and a
# display server.
#
#   x11      Xvfb with openbox, which gives a work area and focus.
#   wayland  sway with WLR_BACKENDS=headless, which offers layer-shell and ext-idle-notify.
#
# Usage: scripts/linux-desktop-harness.sh {x11|wayland} [command...]
#
# Without a command it runs the desktop integration tests: the ignored tests whose name holds
# `desktop_harness`, marked `#[ignore = "desktop-harness"]`: for now those of logind, of the idle
# time, of the tray and of the notifications
# (crates/bientot-linux/src/{logind,idle,tray,notifications}/native/tests.rs), where the app opens
# its flyout, what it does without a notification server, how a later launch hands over to it, and
# what it records of the input, the lock and the sleep
# (crates/bientot-app/tests/desktop_harness_{flyout,notifications,instance,activity}.rs). CI runs the headless start
# in the x11 session (scripts/measure-performance.sh).
#
# Needs dbus-run-session, python3-dbusmock, a Vulkan driver (Mesa's software one will do), and
# Xvfb, openbox and xdotool or sway, wtype and grim.
set -euo pipefail

usage() {
  echo "usage: $0 {x11|wayland} [command...]" >&2
  exit 2
}

[ $# -ge 1 ] || usage
session=$1
case $session in
  x11 | wayland) ;;
  *) usage ;;
esac

# Everything below runs on a session bus of its own, which dbus-run-session ends with the command.
if [ -z "${BIENTOT_HARNESS_BUS:-}" ]; then
  BIENTOT_HARNESS_BUS=1 exec dbus-run-session -- "$0" "$@"
fi
shift
if [ $# -eq 0 ]; then
  set -- cargo test --workspace --locked -- --ignored desktop_harness
fi

work=$(mktemp -d)
pids=()
cleanup() {
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# Waits up to 10 s for `check` to pass.
wait_for() {
  local what=$1
  shift
  for _ in $(seq 100); do
    if "$@" > /dev/null 2>&1; then return 0; fi
    sleep 0.1
  done
  echo "The harness gave up waiting for ${what}." >&2
  exit 1
}

# A private bus stands in for the system bus, where logind lives.
mapfile -t system_bus < <(dbus-daemon --session --fork --print-address=1 --print-pid=1 2>/dev/null)
pids+=("${system_bus[1]}")
export DBUS_SYSTEM_BUS_ADDRESS=${system_bus[0]}

has_name() {
  dbus-send "--$1" --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus.NameHasOwner "string:$2" | grep -q 'boolean true'
}

# The distribution's Python, which python3-dbusmock installs for, unless HARNESS_PYTHON names another.
python=${HARNESS_PYTHON:-/usr/bin/python3}
"$python" -m dbusmock --template logind > "$work/logind.log" 2>&1 &
pids+=("$!")
"$python" -m dbusmock --template notification_daemon > "$work/notifications.log" 2>&1 &
pids+=("$!")
"$python" -m dbusmock --template "$(dirname "$0")/../crates/bientot-linux/tests/support/status_notifier_watcher.py" \
  > "$work/status-notifier-watcher.log" 2>&1 &
pids+=("$!")
"$python" -m dbusmock --template "$(dirname "$0")/../crates/bientot-linux/tests/support/mutter_idle_monitor.py" \
  > "$work/mutter-idle-monitor.log" 2>&1 &
pids+=("$!")
wait_for "the logind mock" has_name system org.freedesktop.login1
wait_for "the notification mock" has_name session org.freedesktop.Notifications
wait_for "the StatusNotifierWatcher mock" has_name session org.kde.StatusNotifierWatcher
wait_for "the idle monitor mock" has_name session org.gnome.Mutter.IdleMonitor

case $session in
  x11)
    has_window_manager() {
      xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id'
    }
    unset WAYLAND_DISPLAY
    Xvfb -displayfd 3 -screen 0 1920x1080x24 -nolisten tcp 3> "$work/display" \
      > "$work/xvfb.log" 2>&1 &
    pids+=("$!")
    wait_for "Xvfb" test -s "$work/display"
    DISPLAY=:$(cat "$work/display")
    export DISPLAY
    openbox > "$work/openbox.log" 2>&1 &
    pids+=("$!")
    wait_for "openbox" has_window_manager
    ;;
  wayland)
    unset DISPLAY
    XDG_RUNTIME_DIR=$work/runtime
    export XDG_RUNTIME_DIR
    mkdir -m 700 "$XDG_RUNTIME_DIR"
    printf 'output HEADLESS-1 resolution 1920x1080\nxwayland disable\n' > "$work/sway.conf"
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
      sway -c "$work/sway.conf" > "$work/sway.log" 2>&1 &
    pids+=("$!")
    wait_for "sway" compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock"
    socket=$(compgen -G "$XDG_RUNTIME_DIR/wayland-*.lock")
    WAYLAND_DISPLAY=$(basename "${socket%.lock}")
    SWAYSOCK=$(compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock")
    export WAYLAND_DISPLAY SWAYSOCK
    ;;
esac

"$@"
