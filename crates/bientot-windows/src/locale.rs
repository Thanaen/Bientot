//! The display languages of Windows (Settings, Time & language, Language & region), most
//! preferred first, as `GetUserPreferredUILanguages` lists them.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::DisplayLanguages;

/// The tags of a list of language names as Windows writes it: each name ended by a nul, and the
/// list by another.
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows display languages read the list outside the tests"
    )
)]
fn names(list: &[u16]) -> Vec<String> {
    list.split(|unit| *unit == 0)
        .take_while(|name| !name.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(test)]
mod tests;
