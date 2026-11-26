//! The notification on a Wayland connection of its own, read on a thread of its own so that each
//! event is dated when it arrives rather than at the next sample.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_callback, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use super::{Notification, TIMEOUT};
use crate::idle::native::Monotonic;
use crate::idle::{IdleReader, MonotonicClock, Opened};

/// The version that adds `get_input_idle_notification`, which ignores idle inhibitors.
const INPUT_IDLE_VERSION: u32 = 2;

/// Opens the notification on the compositor of `WAYLAND_DISPLAY`, when it offers the notifier
/// and a seat.
pub(in crate::idle) fn open() -> Option<Opened> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Listener>(&connection).ok()?;
    let handle = queue.handle();
    let notifier: ExtIdleNotifierV1 = globals.bind(&handle, 1..=INPUT_IDLE_VERSION, ()).ok()?;
    let seat: wl_seat::WlSeat = globals.bind(&handle, 1..=1, ()).ok()?;
    let timeout = TIMEOUT.as_millis() as u32;
    let mut listener = Listener {
        last: Arc::new(Mutex::new(Some(Notification::Resumed(Monotonic.now())))),
    };
    let inhibitor_blind = notifier.version() >= INPUT_IDLE_VERSION;
    let notification = if inhibitor_blind {
        notifier.get_input_idle_notification(timeout, &seat, &handle, ())
    } else {
        notifier.get_idle_notification(timeout, &seat, &handle, ())
    };
    queue.roundtrip(&mut listener).ok()?;
    let last = Arc::clone(&listener.last);
    let stop = Arc::new(AtomicBool::new(false));
    let thread = std::thread::Builder::new()
        .name("bientot-ext-idle".into())
        .spawn({
            let stop = Arc::clone(&stop);
            move || {
                while queue.blocking_dispatch(&mut listener).is_ok() {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                }
                *lock(&listener.last) = None;
            }
        })
        .ok()?;
    Some(Opened {
        reader: Box::new(ExtIdle {
            last,
            connection,
            handle,
            notification,
            stop,
            thread: Some(thread),
        }),
        inhibitor_blind,
    })
}

/// The last event, `None` once the compositor went away.
type Last = Arc<Mutex<Option<Notification>>>;

fn lock(last: &Last) -> std::sync::MutexGuard<'_, Option<Notification>> {
    last.lock().unwrap_or_else(PoisonError::into_inner)
}

struct ExtIdle {
    last: Last,
    connection: Connection,
    handle: QueueHandle<Listener>,
    notification: ExtIdleNotificationV1,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl IdleReader for ExtIdle {
    fn idle(&mut self, now: Duration) -> Option<Duration> {
        lock(&self.last).map(|last| last.idle_at(now))
    }
}

impl Drop for ExtIdle {
    /// Wakes the thread with a round trip of its own queue, after which it sees the stop.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.notification.destroy();
        self.connection.display().sync(&self.handle, ());
        let _ = self.connection.flush();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Listener {
    last: Last,
}

impl Dispatch<ExtIdleNotificationV1, ()> for Listener {
    fn event(
        listener: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let now = Monotonic.now();
        let last = match event {
            ext_idle_notification_v1::Event::Idled => Notification::Idled(now),
            ext_idle_notification_v1::Event::Resumed => Notification::Resumed(now),
            _ => return,
        };
        *lock(&listener.last) = Some(last);
    }
}

wayland_client::delegate_noop!(Listener: ignore ExtIdleNotifierV1);
wayland_client::delegate_noop!(Listener: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Listener: ignore wl_callback::WlCallback);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Listener {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
