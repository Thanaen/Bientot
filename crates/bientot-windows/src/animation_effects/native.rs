//! The query of the setting and the thread hook that sees `WM_SETTINGCHANGE`.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, CallNextHookEx, HC_ACTION, HHOOK, SPI_GETCLIENTAREAANIMATION, SetWindowsHookExW,
    SystemParametersInfoW, UnhookWindowsHookEx, WH_CALLWNDPROC, WM_SETTINGCHANGE,
};
use windows_sys::core::BOOL;

use super::reduces_motion;

/// Whether the « Animation effects » setting is off, read now. A failed query keeps the motion.
pub fn reduce_motion() -> bool {
    reduces_motion(animation_effects())
}

fn animation_effects() -> Option<bool> {
    let mut enabled: BOOL = 0;
    // SAFETY: `enabled` is a live, writable `BOOL`, the type this action fills.
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    (read != 0).then_some(enabled != 0)
}

type ChangeHandler = Box<dyn FnMut()>;

thread_local! {
    /// What the hook of this thread does on each `WM_SETTINGCHANGE`.
    static ON_CHANGE: RefCell<Option<ChangeHandler>> = const { RefCell::new(None) };
}

unsafe extern "system" fn call_window_procedure(
    code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for `HC_ACTION`, `l_param` points to the message about to be delivered.
        let message = unsafe { &*(l_param as *const CWPSTRUCT) }.message;
        if message == WM_SETTINGCHANGE {
            // `try_with`: while the thread exits, its locals may be gone and `with` would panic
            // into Windows.
            let _ = ON_CHANGE.try_with(|handler| {
                if let Ok(mut handler) = handler.try_borrow_mut()
                    && let Some(handler) = handler.as_mut()
                {
                    // A panic must not unwind into Windows.
                    let _ = catch_unwind(AssertUnwindSafe(handler));
                }
            });
        }
    }
    // SAFETY: passes the hook on unchanged; Windows ignores the first argument.
    unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) }
}

/// `WM_SETTINGCHANGE` as the windows of this thread receive it, until dropped. Windows sends it
/// to every top-level window, the notification-area icon's hidden one included, so a change
/// arrives whether or not WorkTime shows a window. One watch per thread.
pub struct SettingChanges(HHOOK);

impl SettingChanges {
    /// Calls `on_change` on this thread for each settings change, before the window procedure
    /// sees it. The setting changed is not told: read it again.
    pub fn watch(on_change: impl FnMut() + 'static) -> io::Result<Self> {
        debug_assert!(
            ON_CHANGE.with_borrow(Option::is_none),
            "one settings watch per thread"
        );
        ON_CHANGE.set(Some(Box::new(on_change)));
        // SAFETY: a thread hook of this process needs no module; the procedure lives as long as
        // the process.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_CALLWNDPROC,
                Some(call_window_procedure),
                ptr::null_mut(),
                GetCurrentThreadId(),
            )
        };
        if hook.is_null() {
            ON_CHANGE.set(None);
            return Err(io::Error::last_os_error());
        }
        Ok(Self(hook))
    }
}

impl Drop for SettingChanges {
    fn drop(&mut self) {
        // SAFETY: the hook came from `SetWindowsHookExW` and is removed once.
        unsafe { UnhookWindowsHookEx(self.0) };
        ON_CHANGE.set(None);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, SendMessageW, WM_NULL,
    };

    use super::*;

    fn window() -> HWND {
        // SAFETY: `STATIC` is a system class; `HWND_MESSAGE` makes the window message-only and the
        // other pointers may be null.
        let window = unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!window.is_null());
        window
    }

    #[test]
    fn settings_changes_reach_the_host_until_the_watch_is_dropped() {
        let window = window();
        let changes = Rc::new(Cell::new(0));
        let counted = changes.clone();
        let watch = SettingChanges::watch(move || counted.set(counted.get() + 1)).unwrap();
        let send = |message| {
            // SAFETY: the window is live and of this thread; neither message reads its params.
            unsafe { SendMessageW(window, message, 0, 0) };
        };

        send(WM_SETTINGCHANGE);
        send(WM_NULL);
        drop(watch);
        send(WM_SETTINGCHANGE);
        // SAFETY: the window was created above on this thread and is destroyed once.
        unsafe { DestroyWindow(window) };

        assert_eq!(changes.get(), 1);
    }

    #[test]
    fn the_setting_reads() {
        assert!(animation_effects().is_some());
    }
}
