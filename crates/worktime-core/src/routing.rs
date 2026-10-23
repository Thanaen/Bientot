//! Which activity source answers for which time, ported from `Services/ActivitySourceRouter.cs`,
//! `Services/ActivitySourceSelectionStartup.cs` and `Services/ActivitySourceSwitch.cs`.
//!
//! The selection ledger records every source the user picked and from when. ActivityWatch is not
//! ported in v1, so the time it was selected for reads as unavailable: a read entirely inside
//! that time keeps the summaries .NET stored for it, since a stored day is only replaced from a
//! source that answered, and a read that also covers built-in time accounts the ActivityWatch
//! part as unknown, as .NET does when the ActivityWatch server is down.

use jiff::Timestamp;
use worktime_domain::accounting::ActivityCoverage;
use worktime_domain::configuration::{SettingsError, WorkTimeSettings};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::sources::{
    ActivitySourceKind, ActivitySourceSelection, SourceFreshness, SourceHealth, SourceQuality,
};
use worktime_domain::time::Clock;
use worktime_store::activity::{ActivityError, ActivitySourceSelectionRepository};
use worktime_store::settings::{SaveSettingsError, SettingsStore};

use crate::sources::{ActivityReadResult, ActivitySource};

/// The source a read names for time before the first selection.
pub const NO_SOURCE_ID: &str = "activity-source:none";
const NO_SOURCE_REASON: &str = "no-activity-source-selected";
/// The source a read names for time ActivityWatch was selected for.
pub const ACTIVITY_WATCH_ID: &str = "activitywatch";
const ACTIVITY_WATCH_REASON: &str = "activitywatch-not-supported";

/// Sends each part of a read to the source selected for it. A read that spans a selection change
/// is split there; no source is read for time before the first selection.
pub struct ActivitySourceRouter<'a> {
    selections: ActivitySourceSelectionRepository<'a>,
    built_in: &'a dyn ActivitySource,
}

impl<'a> ActivitySourceRouter<'a> {
    pub fn new(
        selections: ActivitySourceSelectionRepository<'a>,
        built_in: &'a dyn ActivitySource,
    ) -> Self {
        Self {
            selections,
            built_in,
        }
    }

    fn read_part(
        &self,
        interval: UtcInterval,
        source: ActivitySourceKind,
    ) -> Result<ActivityReadResult, ActivityError> {
        match source {
            ActivitySourceKind::BuiltIn => self.built_in.read(interval),
            ActivitySourceKind::ActivityWatch => Ok(ActivityReadResult::Unavailable {
                health: degraded(ACTIVITY_WATCH_ID, ACTIVITY_WATCH_REASON),
            }),
        }
    }
}

impl ActivitySource for ActivitySourceRouter<'_> {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        let timeline = self
            .selections
            .read_timeline(interval.start(), Some(interval.end()))?;
        match split(interval, &timeline)[..] {
            [] => Ok(ActivityReadResult::Unavailable {
                health: degraded(NO_SOURCE_ID, NO_SOURCE_REASON),
            }),
            [(only, source)] if only == interval => self.read_part(only, source),
            ref parts => {
                let results = parts
                    .iter()
                    .map(|&(part, source)| Ok((part, self.read_part(part, source)?)))
                    .collect::<Result<Vec<_>, ActivityError>>()?;
                Ok(combine(interval, &results))
            }
        }
    }
}

/// The parts of `interval` each selection of `timeline` covers, consecutive selections of one
/// source merged.
fn split(
    interval: UtcInterval,
    timeline: &[ActivitySourceSelection],
) -> Vec<(UtcInterval, ActivitySourceKind)> {
    let mut parts: Vec<(UtcInterval, ActivitySourceKind)> = Vec::new();
    for (index, selection) in timeline.iter().enumerate() {
        let start = selection.effective_from.max(interval.start());
        let end = timeline
            .get(index + 1)
            .map_or(interval.end(), |next| next.effective_from);
        let Ok(part) = UtcInterval::new(start, end) else {
            continue;
        };
        match parts.last_mut() {
            Some((previous, source)) if *source == selection.source => {
                *previous = UtcInterval::new(previous.start(), end)
                    .expect("a later selection ends after the part before it starts");
            }
            _ => parts.push((part, selection.source)),
        }
    }
    parts
}

/// Keeps each part's coverage within that part, since a source can answer with more than it was
/// asked for. The source selected at the end of the read answers for health, because it is the
/// one the user can act on now.
fn combine(
    bounds: UtcInterval,
    results: &[(UtcInterval, ActivityReadResult)],
) -> ActivityReadResult {
    let segments = results.iter().flat_map(|(part, result)| match result {
        ActivityReadResult::Snapshot { coverage, .. } => {
            ActivityCoverage::new(*part, coverage.segments().iter().cloned())
                .segments()
                .to_vec()
        }
        ActivityReadResult::Unavailable { .. } => Vec::new(),
    });
    let (_, current) = results.last().expect("a split read has several parts");
    ActivityReadResult::Snapshot {
        coverage: ActivityCoverage::new(bounds, segments),
        health: current.health().clone(),
    }
}

fn degraded(source_id: &str, reason: &str) -> SourceHealth {
    SourceHealth::degraded(
        source_id,
        SourceFreshness::Unavailable,
        SourceQuality::Incomplete,
        reason,
        None,
    )
    .expect("an unavailable source with a reason is degraded")
}

/// The source `settings` select. Settings without one predate the built-in source and read
/// ActivityWatch.
pub fn selected(settings: &WorkTimeSettings) -> ActivitySourceKind {
    settings
        .activity_source
        .unwrap_or(ActivitySourceKind::ActivityWatch)
}

#[derive(Debug)]
pub enum StartupSelectionError {
    Selections(ActivityError),
    Settings(SaveSettingsError),
}

impl std::fmt::Display for StartupSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Selections(error) => error.fmt(f),
            Self::Settings(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StartupSelectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Selections(error) => Some(error),
            Self::Settings(error) => Some(error),
        }
    }
}

impl From<ActivityError> for StartupSelectionError {
    fn from(error: ActivityError) -> Self {
        Self::Selections(error)
    }
}

/// Runs before any activity read and returns the settings to start with.
///
/// Settings without a source were written by an onboarding completed with ActivityWatch, before
/// the built-in source existed, so ActivityWatch is recorded for all earlier history unless a
/// selection already exists. The settings then mirror the latest selection, which the ledger
/// keeps authoritative; a save that fails to write leaves the mirror to the next start. Settings
/// that name a source over a ledger without any selection, such as a database deleted on its
/// own, record that source from now, so earlier time stays unknown.
pub fn record_startup_selection(
    settings: WorkTimeSettings,
    selections: &ActivitySourceSelectionRepository,
    settings_store: &SettingsStore,
    clock: &dyn Clock,
) -> Result<WorkTimeSettings, StartupSelectionError> {
    if let Some(named) = settings.activity_source {
        if selections
            .read_timeline(ActivitySourceSelection::LEGACY_EPOCH, None)?
            .is_empty()
        {
            selections.append(ActivitySourceSelection {
                effective_from: clock.now(),
                source: named,
            })?;
        }
        return Ok(settings);
    }

    selections.append_legacy_selection()?;
    let latest = selections
        .read_timeline(ActivitySourceSelection::LEGACY_EPOCH, None)?
        .last()
        .expect("the legacy selection was just ensured")
        .source;
    let mirrored = WorkTimeSettings {
        activity_source: Some(latest),
        ..settings
    };
    match settings_store.save(&mirrored) {
        Ok(()) | Err(SaveSettingsError::Io(_)) => Ok(mirrored),
        Err(error) => Err(StartupSelectionError::Settings(error)),
    }
}

#[derive(Debug)]
pub enum SwitchError<E> {
    /// Nothing was recorded or saved.
    InvalidSettings(Vec<SettingsError>),
    /// Includes a failure to record the selection that puts the ledger back after a failed save.
    Selections(ActivityError),
    /// The ledger was put back on the source the settings still name.
    Save(E),
}

impl<E: std::fmt::Display> std::fmt::Display for SwitchError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSettings(_) => f.write_str("Invalid workday settings."),
            Self::Selections(error) => error.fmt(f),
            Self::Save(error) => error.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for SwitchError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidSettings(_) => None,
            Self::Selections(error) => Some(error),
            Self::Save(error) => Some(error),
        }
    }
}

/// Changes the selected activity source from now on, in the selection ledger and in the settings.
pub struct ActivitySourceSwitch<'a> {
    selections: ActivitySourceSelectionRepository<'a>,
    clock: &'a dyn Clock,
}

impl<'a> ActivitySourceSwitch<'a> {
    pub fn new(selections: ActivitySourceSelectionRepository<'a>, clock: &'a dyn Clock) -> Self {
        Self { selections, clock }
    }

    /// Records a selection of `source` effective now, then saves `current` changed by
    /// `also_apply` and naming `source` through `save`. When `current` already names `source`,
    /// only the change is saved and nothing is recorded. A failed save records the source
    /// `current` still names again, effective from then, so that the ledger and the settings
    /// agree.
    pub fn select<T, E>(
        &self,
        current: &WorkTimeSettings,
        source: ActivitySourceKind,
        also_apply: impl FnOnce(WorkTimeSettings) -> WorkTimeSettings,
        save: impl FnOnce(WorkTimeSettings) -> Result<T, E>,
    ) -> Result<T, SwitchError<E>> {
        let next = WorkTimeSettings {
            activity_source: Some(source),
            ..also_apply(current.clone())
        };
        if current.activity_source == Some(source) {
            return save(next).map_err(SwitchError::Save);
        }
        let errors = next.validate();
        if !errors.is_empty() {
            return Err(SwitchError::InvalidSettings(errors));
        }

        self.record(self.clock.now(), source)?;
        save(next).or_else(|error| {
            self.record(self.clock.now(), selected(current))?;
            Err(SwitchError::Save(error))
        })
    }

    fn record<E>(
        &self,
        effective_from: Timestamp,
        source: ActivitySourceKind,
    ) -> Result<(), SwitchError<E>> {
        self.selections
            .append(ActivitySourceSelection {
                effective_from,
                source,
            })
            .map_err(SwitchError::Selections)
    }
}

#[cfg(test)]
mod tests;
