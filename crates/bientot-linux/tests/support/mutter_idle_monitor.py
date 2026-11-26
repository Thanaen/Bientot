"""Mutter's idle monitor for the headless desktop harness (scripts/linux-desktop-harness.sh).

A python-dbusmock template, loaded with `python3 -m dbusmock --template <this file>`. Its core
monitor answers `GetIdletime` with the milliseconds since the last input, which no device gives
here: the mock counts from its start, and a test sets the idle time with the mock interface's
`SetIdletime(milliseconds)`, after which it keeps growing as GNOME's would without input.
"""

import time

import dbus
from dbusmock import MOCK_IFACE

BUS_NAME = "org.gnome.Mutter.IdleMonitor"
MAIN_OBJ = "/org/gnome/Mutter/IdleMonitor/Core"
MAIN_IFACE = "org.gnome.Mutter.IdleMonitor"
SYSTEM_BUS = False


def load(mock, parameters):
    mock.last_input = time.monotonic()


@dbus.service.method(MAIN_IFACE, out_signature="t")
def GetIdletime(self):
    return dbus.UInt64(round((time.monotonic() - self.last_input) * 1000))


@dbus.service.method(MOCK_IFACE, in_signature="t")
def SetIdletime(self, milliseconds):
    self.last_input = time.monotonic() - milliseconds / 1000
