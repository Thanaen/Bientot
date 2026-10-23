//! The last-input query of the .NET `LastInputPort`.

#![allow(unsafe_code)]

use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use worktime_domain::sources::LastInputReading;

use super::{LastInputProbe, reading_from_ticks};

/// `GetLastInputInfo` for the session this process runs in.
pub struct SystemLastInput;

impl LastInputProbe for SystemLastInput {
    fn read(&self) -> Option<LastInputReading> {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // The input time is read before the tick count so that input arriving between the two
        // calls can never place the last input after the tick count, which would read as about
        // 49.7 days of idle time.
        // SAFETY: `info` is a live, writable `LASTINPUTINFO` whose `cbSize` is its own size, as
        // the call requires.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // SAFETY: takes no arguments and cannot fail.
        let tick_count = unsafe { GetTickCount64() };
        Some(reading_from_ticks(tick_count, info.dwTime))
    }
}
