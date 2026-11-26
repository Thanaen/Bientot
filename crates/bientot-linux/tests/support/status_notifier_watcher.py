"""A StatusNotifierWatcher for the headless desktop harness (scripts/linux-desktop-harness.sh).

A python-dbusmock template, loaded with `python3 -m dbusmock --template <this file>`. It keeps
the items that register, says a host is present as KDE Plasma's and GNOME's watchers always do,
and emits the watcher's signals. It draws nothing: a test reads the registered items from the
`RegisteredStatusNotifierItems` property and clicks one by calling `Activate(x, y)` on it, as a
host does.
"""

import dbus

BUS_NAME = "org.kde.StatusNotifierWatcher"
MAIN_OBJ = "/StatusNotifierWatcher"
MAIN_IFACE = "org.kde.StatusNotifierWatcher"
SYSTEM_BUS = False


def load(mock, parameters):
    mock.AddProperties(
        MAIN_IFACE,
        {
            "RegisteredStatusNotifierItems": dbus.Array([], signature="s"),
            "IsStatusNotifierHostRegistered": True,
            "ProtocolVersion": dbus.Int32(0),
        },
    )


@dbus.service.method(MAIN_IFACE, in_signature="s", sender_keyword="sender")
def RegisterStatusNotifierItem(self, service, sender):
    # An item names its bus name, or only its object path on its sender's connection.
    item = service if not service.startswith("/") else sender + service
    items = list(self.Get(MAIN_IFACE, "RegisteredStatusNotifierItems"))
    if item not in items:
        items.append(item)
        self.UpdateProperties(MAIN_IFACE, {"RegisteredStatusNotifierItems": dbus.Array(items, signature="s")})
    self.EmitSignal(MAIN_IFACE, "StatusNotifierItemRegistered", "s", [item])


@dbus.service.method(MAIN_IFACE, in_signature="s")
def RegisterStatusNotifierHost(self, service):
    self.EmitSignal(MAIN_IFACE, "StatusNotifierHostRegistered", "", [])
