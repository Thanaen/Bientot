//! The X11 `MIT-SCREEN-SAVER` extension: the milliseconds since the last input the X server saw.

use std::time::Duration;

use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::screensaver::{self, ConnectionExt as _};
use x11rb::protocol::xproto::Window;
use x11rb::rust_connection::RustConnection;

use super::IdleReader;

/// The extension on the display of `DISPLAY`, on a connection of its own.
pub(super) fn open() -> Option<XScreenSaver> {
    let (connection, screen) = RustConnection::connect(None).ok()?;
    connection
        .extension_information(screensaver::X11_EXTENSION_NAME)
        .ok()??;
    let root = connection.setup().roots.get(screen)?.root;
    let mut reader = XScreenSaver { connection, root };
    reader.idle(Duration::ZERO)?;
    Some(reader)
}

pub(super) struct XScreenSaver {
    connection: RustConnection,
    root: Window,
}

impl IdleReader for XScreenSaver {
    fn idle(&mut self, _now: Duration) -> Option<Duration> {
        let info = self
            .connection
            .screensaver_query_info(self.root)
            .ok()?
            .reply()
            .ok()?;
        Some(Duration::from_millis(info.ms_since_user_input.into()))
    }
}
