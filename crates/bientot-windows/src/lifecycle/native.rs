//! The message-only window that receives the session and power notifications, and the WTS
//! query of the current session state.
//!
//! A message-only window gets no broadcasts, so it does not get the `WM_POWERBROADCAST` the
//! .NET main window receives as a top-level window. `RegisterSuspendResumeNotification` sends
//! the same suspend and resume messages to this window instead.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::mpsc;
use std::thread::JoinHandle;

use bientot_domain::accounting::SessionState;
use bientot_domain::time::{Clock, TimeZoneProvider};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    HPOWERNOTIFY, RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification,
};
use windows_sys::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, ProcessIdToSessionId, WTS_CURRENT_SERVER_HANDLE,
    WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTSFreeMemory, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSRegisterSessionNotification, WTSSessionInfoEx,
    WTSUnRegisterSessionNotification,
};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, PostMessageW,
    PostQuitMessage, RegisterClassExW, WM_CLOSE, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE,
    WNDCLASSEXW, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};
use windows_sys::core::PWSTR;

use super::{LifecycleMessages, LifecycleTransition, WindowMessage, session_state};

const _: () = {
    assert!(super::WM_WTSSESSION_CHANGE == WM_WTSSESSION_CHANGE);
    assert!(super::WTS_SESSION_LOCK == WTS_SESSION_LOCK as usize);
    assert!(super::WTS_SESSION_UNLOCK == WTS_SESSION_UNLOCK as usize);
    assert!(super::WM_POWERBROADCAST == WM_POWERBROADCAST);
    assert!(super::PBT_APMSUSPEND == PBT_APMSUSPEND as usize);
    assert!(super::PBT_APMRESUMEAUTOMATIC == PBT_APMRESUMEAUTOMATIC as usize);
    assert!(super::WTS_SESSIONSTATE_LOCK == WTS_SESSIONSTATE_LOCK as i32);
    assert!(super::WTS_SESSIONSTATE_UNLOCK == WTS_SESSIONSTATE_UNLOCK as i32);
};

const CLASS_NAME: *const u16 = windows_sys::w!("WorkTimeLifecycle");

fn current_session_id() -> io::Result<u32> {
    let mut session_id = 0;
    // SAFETY: `session_id` is a live, writable local; the process id is this process's own.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(session_id)
}

/// The state of this process's session, unknown when Windows cannot tell.
pub fn current_session_state() -> SessionState {
    session_state(current_session_id().ok().and_then(session_flags))
}

fn session_flags(session_id: u32) -> Option<i32> {
    let mut buffer: PWSTR = ptr::null_mut();
    let mut returned_bytes = 0u32;
    // SAFETY: both out pointers are live, writable locals. A buffer Windows allocates is freed
    // below.
    let queried = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session_id,
            WTSSessionInfoEx,
            &mut buffer,
            &mut returned_bytes,
        )
    };
    if queried == 0 || buffer.is_null() {
        return None;
    }
    let flags = (returned_bytes as usize >= size_of::<WTSINFOEXW>())
        .then(|| {
            // SAFETY: the buffer holds at least one `WTSINFOEXW`, as checked; its alignment is not
            // documented, so it is read unaligned.
            let information = unsafe { buffer.cast::<WTSINFOEXW>().read_unaligned() };
            if information.Level != 1 {
                return None;
            }
            // SAFETY: level 1 means the union holds a `WTSINFOEX_LEVEL1_W`.
            let level1 = unsafe { information.Data.WTSInfoExLevel1 };
            (level1.SessionId == session_id).then_some(level1.SessionFlags)
        })
        .flatten();
    // SAFETY: `buffer` was allocated by the query above and is not used after this.
    unsafe { WTSFreeMemory(buffer.cast()) };
    flags
}

type MessageHandler = Box<dyn FnMut(WindowMessage)>;

thread_local! {
    /// What the window procedure does with each message of the window on this thread.
    static ON_MESSAGE: RefCell<Option<MessageHandler>> = const { RefCell::new(None) };
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if message == WM_CLOSE {
        // SAFETY: ends the message loop of this thread, which owns the window.
        unsafe { PostQuitMessage(0) };
        return 0;
    }
    ON_MESSAGE.with(|handler| {
        if let Ok(mut handler) = handler.try_borrow_mut()
            && let Some(handler) = handler.as_mut()
        {
            // A panic must not unwind into Windows; the message is dropped, as the .NET hook
            // swallows a failing callback.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                handler(WindowMessage {
                    message,
                    w_param,
                    l_param,
                })
            }));
        }
    });
    // SAFETY: forwards the message this procedure received, unchanged.
    unsafe { DefWindowProcW(window, message, w_param, l_param) }
}

struct Window(HWND);

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: the window was created on this thread and is destroyed once.
        unsafe { DestroyWindow(self.0) };
    }
}

struct SessionNotifications(HWND);

impl Drop for SessionNotifications {
    fn drop(&mut self) {
        // SAFETY: the window is registered and still exists: it is dropped after this.
        unsafe { WTSUnRegisterSessionNotification(self.0) };
    }
}

struct PowerNotifications(HPOWERNOTIFY);

impl Drop for PowerNotifications {
    fn drop(&mut self) {
        // SAFETY: the handle came from `RegisterSuspendResumeNotification` and is released once.
        unsafe { UnregisterSuspendResumeNotification(self.0) };
    }
}

/// Fields drop in order: both registrations are released before the window is destroyed, and a
/// failed registration releases the ones before it.
struct Registrations {
    _power: PowerNotifications,
    _session: SessionNotifications,
    window: Window,
}

fn open_window() -> io::Result<Registrations> {
    // SAFETY: a null name asks for the module of this executable, which outlives the window.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_procedure),
        hInstance: instance,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: `class` is a complete `WNDCLASSEXW` whose name is a static NUL-terminated string.
    // The class stays registered for the life of the process, so a second window reuses it.
    if unsafe { RegisterClassExW(&class) } == 0
        // SAFETY: reads this thread's last error, set by the call above.
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the class is registered above with this module, `HWND_MESSAGE` makes the window
    // message-only, and the other pointers may be null.
    let window = unsafe {
        CreateWindowExW(
            0,
            CLASS_NAME,
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    let window = Window(window);
    // SAFETY: `window` is a live window of this thread.
    if unsafe { WTSRegisterSessionNotification(window.0, NOTIFY_FOR_THIS_SESSION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let session = SessionNotifications(window.0);
    // SAFETY: `window` is a live window of this thread, named as a window handle by the flag.
    let power = unsafe { RegisterSuspendResumeNotification(window.0, DEVICE_NOTIFY_WINDOW_HANDLE) };
    if power == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Registrations {
        _power: PowerNotifications(power),
        _session: session,
        window,
    })
}

fn pump_messages() {
    let mut message = MSG::default();
    // SAFETY: `message` is a live, writable `MSG`; a null window reads every message of this
    // thread. The loop ends on `WM_QUIT` (0) or an error (-1).
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: `message` was just filled by `GetMessageW`.
        unsafe { DispatchMessageW(&message) };
    }
}

/// The lifecycle notifications of this session, received until dropped. The GPUI app does not
/// lend its window procedure, so this window is owned here and works with or without a visible
/// window.
pub struct LifecycleWindow {
    window: usize,
    thread: Option<JoinHandle<()>>,
}

impl LifecycleWindow {
    /// Opens the window and calls `on_transition` on the window's thread for each lock and
    /// unlock of this session and each suspend and automatic resume, stamped as they arrive.
    pub fn open<C, Z>(
        clock: C,
        zones: Z,
        mut on_transition: impl FnMut(LifecycleTransition) + Send + 'static,
    ) -> io::Result<Self>
    where
        C: Clock + Send + 'static,
        Z: TimeZoneProvider + Send + 'static,
    {
        let messages = LifecycleMessages::new(current_session_id()?, clock, zones);
        let (opened, opening) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("worktime-lifecycle".into())
            .spawn(move || {
                ON_MESSAGE.set(Some(Box::new(move |message| {
                    if let Some(transition) = messages.transition(message) {
                        on_transition(transition);
                    }
                })));
                match open_window() {
                    Ok(registrations) => {
                        let _ = opened.send(Ok(registrations.window.0 as usize));
                        pump_messages();
                        ON_MESSAGE.set(None);
                        drop(registrations);
                    }
                    Err(error) => {
                        ON_MESSAGE.set(None);
                        let _ = opened.send(Err(error));
                    }
                }
            })?;
        match opening.recv() {
            Ok(Ok(window)) => Ok(Self {
                window,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(io::Error::other(
                    "the lifecycle window thread ended before opening",
                ))
            }
        }
    }
}

impl Drop for LifecycleWindow {
    fn drop(&mut self) {
        // SAFETY: posting touches no memory of this process; the window lives until its thread
        // reads this message.
        let posted = unsafe { PostMessageW(self.window as HWND, WM_CLOSE, 0, 0) } != 0;
        if let Some(thread) = self.thread.take()
            && posted
        {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use bientot_domain::time::{FixedTimeZoneProvider, FrozenClock};
    use jiff::Timestamp;
    use jiff::tz::TimeZone;

    use super::*;
    use crate::PowerState;
    use crate::lifecycle::LifecycleChange;

    fn post(window: &LifecycleWindow, message: u32, w_param: usize, l_param: isize) {
        // SAFETY: posting touches no memory of this process.
        assert_ne!(
            unsafe { PostMessageW(window.window as HWND, message, w_param, l_param) },
            0
        );
    }

    #[test]
    fn messages_of_this_session_reach_the_host_until_the_window_is_dropped() {
        let at: Timestamp = "2026-08-30T09:42:17Z".parse().unwrap();
        let (sender, received) = mpsc::channel();
        let window = LifecycleWindow::open(
            FrozenClock::new(at),
            FixedTimeZoneProvider::new("Romance Standard Time", TimeZone::UTC),
            move |transition| {
                let _ = sender.send(transition);
            },
        )
        .unwrap();
        let session = current_session_id().unwrap();

        post(&window, WM_WTSSESSION_CHANGE, 7, session as isize);
        post(
            &window,
            WM_WTSSESSION_CHANGE,
            8,
            session.wrapping_add(1) as isize,
        );
        post(&window, WM_POWERBROADCAST, 4, 0);
        post(&window, WM_POWERBROADCAST, 7, 0);
        drop(window);

        assert_eq!(
            received.iter().collect::<Vec<_>>(),
            vec![
                LifecycleTransition {
                    change: LifecycleChange::Session(SessionState::Locked),
                    occurred_at: at,
                    time_zone_id: "Romance Standard Time".into(),
                },
                LifecycleTransition {
                    change: LifecycleChange::Power(PowerState::Suspended),
                    occurred_at: at,
                    time_zone_id: "Romance Standard Time".into(),
                },
            ]
        );
    }

    #[test]
    fn a_failed_session_query_reads_as_unknown() {
        assert_eq!(
            session_state(session_flags(u32::MAX - 1)),
            SessionState::Unknown
        );
    }
}
