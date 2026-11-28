//! Microsoft Store updates, ported from `Updates/StoreContextUpdatePort.cs`. Only the MSIX build
//! has a Store to ask; on Windows [`StoreUpdates`] asks it through WinRT's `StoreContext`.
//!
//! The Store closes a packaged desktop app to install its update, and starts it again only when
//! the app registered for a restart beforehand: [`restart_command_line`] spells the arguments of
//! that restart.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::{StoreUpdateOffer, StoreUpdates};

/// `arguments` as one command line that Windows splits back into the same arguments, as
/// `RegisterApplicationRestart` takes them. An argument is quoted only when it must be.
pub fn restart_command_line(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| quoted(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `argument`, quoted when it is empty or holds a space, a tab or a quote. In quotes, a quote is
/// escaped and the backslashes before it or before the closing quote are doubled, as
/// `CommandLineToArgvW` reads them.
fn quoted(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(2 * backslashes + 1));
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
            }
        }
        if character != '\\' {
            quoted.push(character);
        }
    }
    quoted.push_str(&"\\".repeat(2 * backslashes));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(arguments: &[&str]) -> String {
        restart_command_line(&arguments.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn flags_stay_as_they_are() {
        assert_eq!(line(&[]), "");
        assert_eq!(line(&["--open-flyout", "--dark"]), "--open-flyout --dark");
    }

    #[test]
    fn a_folder_with_spaces_is_quoted_and_keeps_its_backslashes() {
        assert_eq!(
            line(&["--data-dir", r"C:\Users\Jean Dupont\Bientôt\"]),
            r#"--data-dir "C:\Users\Jean Dupont\Bientôt\\""#
        );
    }

    #[test]
    fn quotes_and_empty_arguments_survive() {
        assert_eq!(line(&[""]), r#""""#);
        assert_eq!(line(&[r#"a "b"#]), r#""a \"b""#);
        assert_eq!(line(&[r#"a\"b c"#]), r#""a\\\"b c""#);
    }
}
