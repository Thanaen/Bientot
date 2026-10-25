//! What the week's tests share: literal texts, the demo's dates and its ledger.

use std::path::PathBuf;

use bientot_core::demo::DemoScenario;
use jiff::civil::Date;

use crate::demo::{self, DemoHost};
use crate::strings::keep_together;

/// `text` with each `~` as a no-break space.
pub(super) fn nb(text: &str) -> String {
    text.replace('~', &keep_together(" "))
}

pub(super) fn date(month: i8, day: i8) -> Date {
    jiff::civil::date(2026, month, day)
}

/// The throwaway folder of a demo's ledger, removed when the test ends.
pub(super) struct Throwaway(PathBuf);

impl Throwaway {
    pub(super) fn open(scenario: DemoScenario) -> (DemoHost, Self) {
        let directory = demo::throwaway_directory();
        let host = DemoHost::open(scenario, &directory).unwrap();
        (host, Self(directory))
    }
}

impl Drop for Throwaway {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
