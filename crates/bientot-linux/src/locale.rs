//! The language of the messages, as POSIX sets it from the environment: `LC_ALL`, else
//! `LC_MESSAGES`, else `LANG`, the first one set and not empty.

use std::ffi::OsString;

use bientot_platform::locale::SystemLocale;

/// The locale of this process's environment.
pub struct EnvironmentLocale;

impl SystemLocale for EnvironmentLocale {
    fn languages(&self) -> Vec<String> {
        messages_language(|name| std::env::var_os(name))
            .into_iter()
            .collect()
    }
}

/// The language of the messages in an environment that `variable` reads, as a tag such as
/// `fr-FR`. `None` when no variable is set, or when the one that decides names no language: `C`
/// and `POSIX` among them, which give the messages in English. A variable later in the order
/// never stands in for one that decides.
pub fn messages_language(variable: impl Fn(&str) -> Option<OsString>) -> Option<String> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| variable(name).filter(|value| !value.is_empty()))?;
    tag(locale.to_str()?)
}

/// A locale named `language[_territory][.codeset][@modifier]`, such as `fr_FR.UTF-8@euro`, as
/// `fr-FR`.
fn tag(locale: &str) -> Option<String> {
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    let (language, territory) = name.split_once('_').unwrap_or((name, ""));
    let is_language = (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_alphabetic());
    if !is_language {
        return None;
    }
    let language = language.to_ascii_lowercase();
    let is_territory =
        !territory.is_empty() && territory.bytes().all(|byte| byte.is_ascii_alphanumeric());
    Some(if is_territory {
        format!("{language}-{}", territory.to_ascii_uppercase())
    } else {
        language
    })
}

#[cfg(test)]
mod tests;
