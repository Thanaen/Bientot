//! What the tests of the form share: literal texts, instants, and a demo's ledger.

use std::path::PathBuf;

use worktime_core::demo::DemoScenario;

use super::*;
use crate::demo::{self, DemoHost};
use crate::week::WeekState;

pub(super) fn nb(text: &str) -> String {
    text.replace('~', "\u{a0}")
}

pub(super) fn id(text: &str) -> AdjustmentId {
    AdjustmentId::new(text).unwrap()
}

pub(super) fn seeded_at(
    day: Date,
    zone: TimeZone,
    seed: Seed,
    now: &str,
    language: Language,
) -> AdjustForm {
    AdjustForm::new(
        Target {
            workday: LocalWorkday::new(day, "Test/Zone"),
            zone,
        },
        seed,
        id("correction-1"),
        utc(now),
        language,
    )
}

pub(super) fn utc(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// A declaration on `slot` alone, with no day around it.
pub(super) fn on_slot(slot: UtcInterval) -> Seed {
    Seed::DeclareWork(Unrecorded::new(vec![slot], Vec::new()))
}

pub(super) fn interval(start: &str, end: &str) -> UtcInterval {
    UtcInterval::new(utc(start), utc(end)).unwrap()
}

/// A demo's ledger in a throwaway folder, removed when the test ends.
pub(super) struct Demo {
    pub(super) host: DemoHost,
    pub(super) directory: PathBuf,
}

impl Drop for Demo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

impl Demo {
    pub(super) fn open() -> (Self, WeekState) {
        let directory = demo::throwaway_directory();
        let mut demo = Self {
            host: DemoHost::open(DemoScenario::HistoryWeek, &directory).unwrap(),
            directory,
        };
        demo.host.refresh(Language::French).unwrap();
        let state = WeekState::new(demo.host.week().unwrap(), Language::French);
        (demo, state)
    }

    pub(super) fn form(&self, day: Date, id_text: &str) -> AdjustForm {
        self.seeded(day, Seed::Blank, id_text)
    }

    pub(super) fn seeded(&self, day: Date, seed: Seed, id_text: &str) -> AdjustForm {
        AdjustForm::new(
            Target {
                workday: LocalWorkday::new(day, "UTC"),
                zone: TimeZone::UTC,
            },
            seed,
            id(id_text),
            self.host.correction_time(),
            Language::French,
        )
    }

    /// What the window does: record the correction, refresh, then report the result.
    pub(super) fn save(&mut self, form: &mut AdjustForm) -> Result<Option<Closed>, Key> {
        let adjustment = form.submit().unwrap();
        let result = self.host.save_adjustment(&adjustment).map(|_| true);
        self.host.refresh(Language::French).unwrap();
        let closed = form.finished(result);
        result.map(|_| closed)
    }
}
