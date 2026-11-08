//! The module side of ABI 1 (contract `host-abi.md`): buffers cross the boundary as a pointer and
//! a length into the module's memory, allocated by the module's own allocator. Used by
//! [`crate::export!`]; nothing here is meant to be called by a plugin.

#[cfg(target_arch = "wasm32")]
use std::alloc::Layout;
use std::sync::Once;

use bientot_plugin_api::Event;

use crate::Plugin;

pub const ABI: i32 = bientot_plugin_api::ABI_VERSION;

#[cfg(target_arch = "wasm32")]
/// A buffer of `len` bytes for the host, or `0` when `len` is negative or memory ran out. An
/// empty buffer gets a dangling, non-zero pointer that [`free`] ignores.
pub fn alloc(len: i32) -> i32 {
    let Ok(len) = usize::try_from(len) else {
        return 0;
    };
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr() as usize as i32;
    }
    let Ok(layout) = Layout::array::<u8>(len) else {
        return 0;
    };
    // SAFETY: `layout` has a non-zero size.
    let ptr = unsafe { std::alloc::alloc(layout) };
    ptr as usize as i32
}

#[cfg(target_arch = "wasm32")]
/// Frees a buffer made by [`alloc`].
///
/// # Safety
///
/// `ptr` and `len` must be those of a buffer [`alloc`] returned and that was not freed yet.
pub unsafe fn free(ptr: i32, len: i32) {
    let Ok(len) = usize::try_from(len) else {
        return;
    };
    if len == 0 || ptr == 0 {
        return;
    }
    // SAFETY: the caller guarantees the buffer came from `alloc` with this length.
    unsafe { std::alloc::dealloc(ptr as usize as *mut u8, Layout::array::<u8>(len).unwrap()) }
}

#[cfg(target_arch = "wasm32")]
/// Takes ownership of a buffer made by [`alloc`] and filled by the host.
///
/// # Safety
///
/// As for [`free`]; the buffer must also be initialized over its whole length.
pub unsafe fn take(ptr: i32, len: i32) -> Vec<u8> {
    let Ok(len) = usize::try_from(len) else {
        return Vec::new();
    };
    if len == 0 || ptr == 0 {
        return Vec::new();
    }
    // SAFETY: `alloc` made this buffer with the global allocator, `len` bytes aligned to 1, which
    // is the layout of a `Vec<u8>` of capacity `len`.
    unsafe { Vec::from_raw_parts(ptr as usize as *mut u8, len, len) }
}

/// Splits the `(ptr << 32) | len` an export or an import returns.
pub fn split(packed: i64) -> (i32, i32) {
    ((packed >> 32) as i32, packed as u32 as i32)
}

/// Reads one event and hands it to the plugin. The answer is always empty: the host reads no
/// field of it in api 1.0.
pub fn handle<P: Plugin>(plugin: &mut P, bytes: &[u8]) -> i64 {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            // The module traps right after: the message is the only trace in the plugin's log.
            crate::log::error(&format!("panic: {info}"));
        }))
    });
    match serde_json::from_slice::<Event>(bytes) {
        Ok(event) => plugin.handle(event),
        Err(error) => crate::log::warn(&format!("an event did not read: {error}")),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_answers_split() {
        assert_eq!(split((1234_i64 << 32) | 56), (1234, 56));
        assert_eq!(split(0), (0, 0));
        assert_eq!(split((7_i64 << 32) | 0xFFFF_FFFF), (7, -1));
    }
}
