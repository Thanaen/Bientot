//! The registry and shell calls of the portable registration.

#![allow(unsafe_code)]

use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::HSTRING;

use super::{PORTABLE_ACTIVATOR, PORTABLE_AUMID, RegistrationPlan, keys};
use crate::registry;

impl RegistrationPlan {
    /// Registers the AppUserModelID and its activator, then makes it this process's own. Each
    /// launch writes every value again, since the ZIP may have moved since the last one.
    pub(crate) fn register(&self) -> windows::core::Result<()> {
        self.write()?;
        // SAFETY: the HSTRING is a NUL-terminated string borrowed for the call.
        unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(&self.aumid)) }
    }

    fn write(&self) -> std::io::Result<()> {
        self.values()
            .iter()
            .try_for_each(|value| registry::set_string(&value.key, value.name, &value.data))
    }
}

/// Deletes the portable AppUserModelID and its activator, as the uninstaller of the installed
/// build does. The next launch of any portable copy registers them again.
pub fn unregister() -> std::io::Result<()> {
    delete(PORTABLE_AUMID, PORTABLE_ACTIVATOR)
}

fn delete(aumid: &str, clsid: u128) -> std::io::Result<()> {
    keys(aumid, clsid)
        .iter()
        .try_for_each(|key| registry::delete_tree(key))
}

#[cfg(test)]
mod tests {
    //! Registers an AppUserModelID and a class of their own, so an installed WorkTime keeps its
    //! registration.

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

    use super::super::RegistryValue;
    use super::*;
    use crate::registry::wide;

    const TEST_AUMID: &str = "Thanaen.WorkTime.Tests.PortableRegistration";
    const TEST_ACTIVATOR: u128 = 0xdc4f50b3_6a61_4b3e_b629_506da080fe7e;

    /// Deletes the test's keys as the uninstaller does, even when an assertion fails.
    struct Cleanup;

    impl Drop for Cleanup {
        fn drop(&mut self) {
            // The test asserts the deletion; a second panic while one unwinds would abort.
            let _ = delete(TEST_AUMID, TEST_ACTIVATOR);
        }
    }

    /// The string `value` holds, or the status of the read.
    fn read(value: &RegistryValue) -> Result<Vec<u16>, WIN32_ERROR> {
        let key = wide(&value.key);
        let name = wide(value.name);
        let mut data = vec![0_u16; 1024];
        let mut size = u32::try_from(data.len() * 2).unwrap();
        // SAFETY: the key path and value name are NUL-terminated UTF-16 strings, and `data`
        // holds `size` writable bytes, all borrowed for the call.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(status);
        }
        data.truncate(size as usize / 2);
        Ok(data)
    }

    #[test]
    fn writing_twice_leaves_the_planned_values_until_cleaned_up() {
        let plan = RegistrationPlan {
            aumid: TEST_AUMID.to_owned(),
            clsid: TEST_ACTIVATOR,
            exe: r"C:\Program Files\WorkTime Tests\worktime.exe".into(),
            icon: r"C:\Program Files\WorkTime Tests\worktime.png".into(),
            data_dir: Some(r"C:\Program Files\WorkTime Tests\data".into()),
        };
        let cleanup = Cleanup;

        plan.write().unwrap();
        plan.write().unwrap();
        for value in plan.values() {
            assert_eq!(read(&value), Ok(wide(&value.data)), "{value:?}");
        }

        drop(cleanup);
        for value in plan.values() {
            assert_eq!(read(&value), Err(ERROR_FILE_NOT_FOUND), "{value:?}");
        }
        delete(TEST_AUMID, TEST_ACTIVATOR).expect("deleting what is gone succeeds");
    }
}
