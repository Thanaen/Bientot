//! The idle monitor of Mutter (GNOME, on Wayland and X11) and of Muffin (Cinnamon): the idle time
//! in milliseconds, from input only, on the session bus.

use std::time::Duration;

use zbus::Connection;

use super::IdleReader;
use crate::dbus::call;

/// One compositor's monitor: the service, its core monitor's object and the interface, which is
/// named after the service.
#[derive(Clone, Copy, Debug)]
pub(super) struct Monitor {
    service: &'static str,
    path: &'static str,
}

pub(super) const MUTTER: Monitor = Monitor {
    service: "org.gnome.Mutter.IdleMonitor",
    path: "/org/gnome/Mutter/IdleMonitor/Core",
};

pub(super) const MUFFIN: Monitor = Monitor {
    service: "org.cinnamon.Muffin.IdleMonitor",
    path: "/org/cinnamon/Muffin/IdleMonitor/Core",
};

/// The monitor, once `GetIdletime` answered.
pub(super) fn open(monitor: Monitor) -> Option<IdleMonitor> {
    let bus = async_io::block_on(Connection::session()).ok()?;
    let mut reader = IdleMonitor { bus, monitor };
    reader.idle(Duration::ZERO)?;
    Some(reader)
}

pub(super) struct IdleMonitor {
    bus: Connection,
    monitor: Monitor,
}

impl IdleReader for IdleMonitor {
    fn idle(&mut self, _now: Duration) -> Option<Duration> {
        let Monitor { service, path } = self.monitor;
        let milliseconds: u64 =
            async_io::block_on(call(&self.bus, service, path, service, "GetIdletime", &())).ok()?;
        Some(Duration::from_millis(milliseconds))
    }
}
