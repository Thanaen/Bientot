//! The query of the display languages.

#![allow(unsafe_code)]

use std::ptr;

use bientot_platform::locale::SystemLocale;
use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

use super::names;

/// The display languages of the signed-in user.
pub struct DisplayLanguages;

impl SystemLocale for DisplayLanguages {
    /// Empty when Windows does not answer.
    fn languages(&self) -> Vec<String> {
        let mut count = 0;
        let mut length = 0;
        // SAFETY: the counts are live locals; a null buffer asks for the length the list needs.
        let sized = unsafe {
            GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, ptr::null_mut(), &mut length)
        };
        if sized == 0 {
            return Vec::new();
        }
        let mut list = vec![0u16; length as usize];
        // SAFETY: `list` holds the `length` units the first call asked for.
        let read = unsafe {
            GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                list.as_mut_ptr(),
                &mut length,
            )
        };
        if read == 0 {
            return Vec::new();
        }
        list.truncate(length as usize);
        names(&list)
    }
}
