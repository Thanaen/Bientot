//! The languages the user set the system to: the display languages on Windows, `LC_ALL`,
//! `LC_MESSAGES` or `LANG` on Linux.

pub trait SystemLocale {
    /// The user's languages for the interface, most preferred first, as tags such as `fr-FR`:
    /// empty when the system names none.
    fn languages(&self) -> Vec<String>;
}

/// A system that names the languages it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InMemoryLocale {
    pub languages: Vec<String>,
}

impl SystemLocale for InMemoryLocale {
    fn languages(&self) -> Vec<String> {
        self.languages.clone()
    }
}
