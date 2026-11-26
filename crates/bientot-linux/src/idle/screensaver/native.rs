//! `GetSessionIdleTime` on the session bus, from a screensaver already running.

use std::time::Duration;

use zbus::Connection;

use super::{Read, UNIT_CHECK_READS, UNIT_CHECK_STEP, Unit, unit};
use crate::dbus::{call, has_owner};
use crate::idle::native::Monotonic;
use crate::idle::{IdleReader, MonotonicClock};

const SERVICE: &str = "org.freedesktop.ScreenSaver";
/// The object, where desktops put it.
const PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];

/// The screensaver, once one of its objects answered the unit check and the answers told the
/// unit. Takes a second.
pub(in crate::idle) fn open() -> Option<ScreenSaver> {
    let bus = async_io::block_on(Connection::session()).ok()?;
    if !async_io::block_on(has_owner(&bus, SERVICE)) {
        return None;
    }
    let (path, value) = PATHS
        .into_iter()
        .find_map(|path| Some((path, idle_time(&bus, path)?)))?;
    let mut reads = Vec::with_capacity(UNIT_CHECK_READS);
    reads.push(Read {
        at: Monotonic.now(),
        value,
    });
    while reads.len() < UNIT_CHECK_READS {
        std::thread::sleep(UNIT_CHECK_STEP);
        let value = idle_time(&bus, path)?;
        reads.push(Read {
            at: Monotonic.now(),
            value,
        });
    }
    let unit = unit(&reads)?;
    Some(ScreenSaver { bus, path, unit })
}

fn idle_time(bus: &Connection, path: &str) -> Option<u32> {
    async_io::block_on(call(bus, SERVICE, path, SERVICE, "GetSessionIdleTime", &())).ok()
}

pub(in crate::idle) struct ScreenSaver {
    bus: Connection,
    path: &'static str,
    unit: Unit,
}

impl IdleReader for ScreenSaver {
    fn idle(&mut self, _now: Duration) -> Option<Duration> {
        idle_time(&self.bus, self.path).map(|value| self.unit.idle(value))
    }
}
