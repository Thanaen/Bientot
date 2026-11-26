//! The Linux backend: the tray icon is a StatusNotifierItem of `bientot-linux`. The other ports
//! answer « unknown » or do nothing, as on a system without a backend, until their lots wire
//! them; the flyout opens where the compositor puts it.

#[path = "null.rs"]
mod null;

use std::rc::Rc;

use bientot_linux::tray::StatusNotifier;
use futures::channel::mpsc;

use super::{Platform, PlatformContext};

pub use null::*;

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform> {
    let (events, tray_events) = mpsc::unbounded();
    let tray = StatusNotifier::new(move |event| {
        events.unbounded_send(event).ok();
    });
    Ok(Platform {
        tray_backend: Some(tray.backend()),
        tray: Rc::new(tray),
        tray_events,
        ..null::build(ctx)?
    })
}
