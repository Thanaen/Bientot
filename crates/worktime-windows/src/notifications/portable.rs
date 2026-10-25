//! The sender the portable build shows its toasts as. Without package identity, Windows shows a
//! toast only for an AppUserModelID registered under `HKEY_CURRENT_USER`, and hands each click to
//! the COM class that registration names, starting the class's `LocalServer32` command when no
//! running WorkTime registered the class.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use worktime_store::data_directory;

use super::ACTIVATOR_ARGUMENT;

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::unregister;

const PORTABLE_AUMID: &str = "Thanaen.WorkTime.Portable";

/// Differs from [`super::PACKAGED_ACTIVATOR`] so that what an unpacked ZIP registers never starts
/// that copy of WorkTime for a click on the package's toasts.
const PORTABLE_ACTIVATOR: u128 = 0x8ddae643_7db0_465f_9139_725c4f57f1ed;

/// The AppUserModelID and toast activator of a WorkTime run without package identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationPlan {
    pub aumid: String,
    pub clsid: u128,
    pub exe: PathBuf,
    pub icon: PathBuf,
    /// The folder a click starts WorkTime on, when not the default one.
    pub data_dir: Option<PathBuf>,
}

/// A string value under `HKEY_CURRENT_USER`. An empty `name` is the key's default value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RegistryValue {
    key: String,
    name: &'static str,
    data: OsString,
}

impl RegistrationPlan {
    /// The portable build at `exe`, shown in Notification Center with the PNG at `icon`. It runs
    /// on `data_dir` when not the default folder, so that a click on one of its toasts reopens
    /// the folder that showed it.
    pub fn portable(exe: PathBuf, icon: PathBuf, data_dir: Option<PathBuf>) -> Self {
        Self {
            aumid: PORTABLE_AUMID.to_owned(),
            clsid: PORTABLE_ACTIVATOR,
            exe,
            icon,
            data_dir,
        }
    }

    #[cfg_attr(
        not(windows),
        allow(
            dead_code,
            reason = "only the registration reads them outside the tests"
        )
    )]
    fn values(&self) -> [RegistryValue; 4] {
        let clsid = braced(self.clsid);
        let [aumid_key, clsid_key] = keys(&self.aumid, self.clsid);
        let mut command = quoted(self.exe.as_os_str());
        if let Some(data_dir) = &self.data_dir {
            command.push(format!(" {} ", data_directory::ARGUMENT));
            command.push(quoted(data_dir.as_os_str()));
        }
        command.push(format!(" {ACTIVATOR_ARGUMENT}"));
        [
            RegistryValue {
                key: aumid_key.clone(),
                name: "DisplayName",
                data: "Bientôt".into(),
            },
            RegistryValue {
                key: aumid_key.clone(),
                name: "IconUri",
                data: self.icon.clone().into(),
            },
            RegistryValue {
                key: aumid_key,
                name: "CustomActivator",
                data: clsid.clone().into(),
            },
            RegistryValue {
                key: format!(r"{clsid_key}\LocalServer32"),
                name: "",
                data: command,
            },
        ]
    }
}

/// The keys of the AppUserModelID `aumid` and of its activator `clsid`, which hold every value
/// of the registration.
fn keys(aumid: &str, clsid: u128) -> [String; 2] {
    [
        format!(r"Software\Classes\AppUserModelId\{aumid}"),
        format!(r"Software\Classes\CLSID\{}", braced(clsid)),
    ]
}

/// `argument` in quotes, as the Windows command line reads it back: the backslashes that end it
/// are doubled so that they do not escape the closing quote, as in `"C:\\"`.
fn quoted(argument: &OsStr) -> OsString {
    let trailing = argument
        .as_encoded_bytes()
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count();
    let mut quoted = OsString::from("\"");
    quoted.push(argument);
    quoted.push("\\".repeat(trailing));
    quoted.push("\"");
    quoted
}

/// The registry spelling of a CLSID, `{6CB667AA-56D8-4F29-B21C-7D6F329B4463}`.
fn braced(clsid: u128) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}",
        clsid >> 96,
        (clsid >> 80) & 0xffff,
        (clsid >> 64) & 0xffff,
        (clsid >> 48) & 0xffff,
        clsid & 0xffff_ffff_ffff,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(key: &str, name: &'static str, data: &str) -> RegistryValue {
        RegistryValue {
            key: key.to_owned(),
            name,
            data: data.into(),
        }
    }

    #[test]
    fn the_portable_build_registers_its_aumid_and_starts_itself_for_a_click() {
        let plan = RegistrationPlan::portable(
            r"C:\Users\Clem\Tools\WorkTime Portable\worktime.exe".into(),
            r"C:\Users\Clem\Tools\WorkTime Portable\worktime.png".into(),
            None,
        );

        let aumid = r"Software\Classes\AppUserModelId\Thanaen.WorkTime.Portable";
        assert_eq!(
            plan.values(),
            [
                value(aumid, "DisplayName", "Bientôt"),
                value(
                    aumid,
                    "IconUri",
                    r"C:\Users\Clem\Tools\WorkTime Portable\worktime.png"
                ),
                value(
                    aumid,
                    "CustomActivator",
                    "{8DDAE643-7DB0-465F-9139-725C4F57F1ED}"
                ),
                value(
                    r"Software\Classes\CLSID\{8DDAE643-7DB0-465F-9139-725C4F57F1ED}\LocalServer32",
                    "",
                    r#""C:\Users\Clem\Tools\WorkTime Portable\worktime.exe" --toast-activated"#
                ),
            ]
        );
    }

    #[test]
    fn a_click_reopens_the_data_folder_the_build_ran_on() {
        let command = |data_dir: &str| {
            let plan = RegistrationPlan::portable(
                r"C:\Tools\worktime.exe".into(),
                r"C:\Tools\worktime.png".into(),
                Some(data_dir.into()),
            );
            plan.values()[3].data.clone()
        };

        assert_eq!(
            command(r"D:\Work Time\data"),
            r#""C:\Tools\worktime.exe" --data-dir "D:\Work Time\data" --toast-activated"#
        );
        assert_eq!(
            command("D:\\"),
            r#""C:\Tools\worktime.exe" --data-dir "D:\\" --toast-activated"#
        );
    }

    #[test]
    fn the_portable_activator_is_not_the_package_one() {
        assert_ne!(PORTABLE_ACTIVATOR, super::super::PACKAGED_ACTIVATOR);
    }

    #[test]
    fn a_clsid_is_spelled_with_braces_and_every_leading_zero() {
        assert_eq!(
            braced(super::super::PACKAGED_ACTIVATOR),
            "{6CB667AA-56D8-4F29-B21C-7D6F329B4463}"
        );
        assert_eq!(
            braced(0x00000a0b_000c_0d00_0000_000000000e0f),
            "{00000A0B-000C-0D00-0000-000000000E0F}"
        );
    }
}
