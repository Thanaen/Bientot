//! The `--demo` launches: a scenario of `worktime_core::demo` run by the real accounting and
//! weekly recalculation over a ledger in a throwaway folder, or the `--data-dir` folder. The demo
//! opens only a folder it created for the same scenario: it refuses one with WorkTime data of
//! its own, such as the user's.

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use worktime_core::accounting::WorkdayAccounting;
use worktime_core::corrections::ManualAdjustmentCommands;
use worktime_core::demo::{DemoEnvironment, DemoScenario};
use worktime_core::weekly::{WeeklyRecalculation, WorkdayRefreshState};
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::adjustments::ManualAdjustment;
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::sources::{ActivitySourceKind, LastInputReading};
use worktime_domain::time::{Clock, FixedTimeZoneProvider, LocalWorkday};
use worktime_store::adjustments::ManualAdjustmentRepository;
use worktime_store::credit::WeeklyCreditRepository;
use worktime_store::data_directory::DataDirectory;
use worktime_store::sessions::SessionEventRepository;
use worktime_store::settings::{SaveSettingsError, SettingsStore};
use worktime_windows::activity::{LastInputProbe, reading_from_ticks};

use crate::ledger;
use crate::strings::{Key, Language};
use crate::week::{WeekCommand, WeekTimeline};
use crate::workday::DayView;

/// Someone at the keyboard all along, for a live day in the tests and screenshots.
pub struct Typing;

impl LastInputProbe for Typing {
    fn read(&self) -> Option<LastInputReading> {
        Some(reading_from_ticks(10_000, 10_000))
    }
}

/// The `--demo <slug>` names, in the order of [`DemoScenario::ALL`]. The first five are the
/// states of the R1 prototype.
const SLUGS: [(DemoScenario, &str); 13] = [
    (DemoScenario::OnTrack, "on-track"),
    (DemoScenario::Warning, "near-limit"),
    (DemoScenario::Late, "late"),
    (DemoScenario::Complete, "goal-reached"),
    (DemoScenario::Morning, "morning"),
    (DemoScenario::Unavailable, "source-unavailable"),
    (DemoScenario::Credit, "credit"),
    (DemoScenario::HistoryWeek, "history-week"),
    (DemoScenario::MorningWarning, "morning-near-limit"),
    (DemoScenario::MorningLate, "morning-late"),
    (DemoScenario::ProblemPeriods, "problem-periods"),
    (DemoScenario::FragmentedDay, "fragmented-day"),
    (
        DemoScenario::ActivityWatchUnavailable,
        "source-activitywatch",
    ),
];

pub fn slug(scenario: DemoScenario) -> &'static str {
    SLUGS
        .iter()
        .find(|(candidate, _)| *candidate == scenario)
        .map(|(_, slug)| *slug)
        .expect("every scenario has a slug")
}

pub fn from_slug(slug: &str) -> Option<DemoScenario> {
    SLUGS
        .iter()
        .find(|(_, candidate)| *candidate == slug)
        .map(|(scenario, _)| *scenario)
}

/// A new folder under the temporary directory, where the .NET app puts its demo data too. A demo
/// on `--data-dir` keeps its folder from one launch to the next instead, as in .NET.
pub fn throwaway_directory() -> PathBuf {
    DataDirectory::throwaway(&std::env::temp_dir(), crate::random_id())
        .path()
        .to_owned()
}

/// Why the demo does not open a folder.
#[derive(Debug)]
pub enum FolderRefusal {
    /// The folder holds a ledger or settings that the demo did not create: the user's, maybe.
    NotDemo(PathBuf),
    /// The folder keeps the demo of the scenario `kept`, whose corrections were recorded at that
    /// scenario's frozen time.
    OtherScenario {
        directory: PathBuf,
        kept: String,
        requested: DemoScenario,
    },
}

impl FolderRefusal {
    /// What the startup failure window says.
    pub fn message(&self) -> Key {
        match self {
            FolderRefusal::NotDemo(_) => Key::StartupFailureDemoDataMessage,
            FolderRefusal::OtherScenario { .. } => Key::StartupFailureDemoScenarioMessage,
        }
    }
}

impl fmt::Display for FolderRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FolderRefusal::NotDemo(directory) => write!(
                f,
                "the folder {} holds WorkTime data that the demo did not create",
                directory.display()
            ),
            FolderRefusal::OtherScenario {
                directory,
                kept,
                requested,
            } => write!(
                f,
                "the demo folder {} keeps the scenario {kept}, not {}",
                directory.display(),
                slug(*requested)
            ),
        }
    }
}

impl std::error::Error for FolderRefusal {}

/// The file a demo writes in its folder: the folder is the demo's, of `scenario`, and `seeded`
/// once the scenario's history is in the ledger.
const MARKER: &str = "demo.json";

struct Marker {
    scenario: String,
    seeded: bool,
}

impl Marker {
    fn read(path: &Path) -> anyhow::Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        let value: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("cannot read {}", path.display()))?;
        match (value["scenario"].as_str(), value["seeded"].as_bool()) {
            (Some(scenario), Some(seeded)) => Ok(Some(Self {
                scenario: scenario.to_owned(),
                seeded,
            })),
            _ => Err(anyhow::anyhow!("cannot read {}", path.display())),
        }
    }

    /// Replaces `path` through a temporary file, so that a crash never leaves half a marker.
    fn write(path: &Path, scenario: DemoScenario, seeded: bool) -> anyhow::Result<()> {
        let text = serde_json::json!({ "scenario": slug(scenario), "seeded": seeded });
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, format!("{text:#}\n"))
            .and_then(|()| std::fs::rename(&temporary, path))
            .with_context(|| format!("cannot write {}", path.display()))
    }
}

/// What [`claim`] found in the demo's folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claimed {
    /// No ledger with the scenario's history yet: the demo seeds one.
    Fresh,
    /// The ledger of earlier launches, with their corrections.
    Kept,
}

/// Makes `directory` the demo's folder of `scenario`, and tells whether its ledger still needs
/// the scenario's history. A seed that did not finish left its marker unseeded, and a ledger
/// deleted by hand left a seeded marker without a ledger: what remains of the ledger is removed
/// and seeded again, the marker unseeded until the seed is done.
fn claim(directory: &DataDirectory, scenario: DemoScenario) -> anyhow::Result<Claimed> {
    let marker = directory.path().join(MARKER);
    let exists = |path: PathBuf| {
        path.try_exists()
            .with_context(|| format!("cannot read {}", path.display()))
    };
    match Marker::read(&marker)? {
        None => {
            if exists(directory.database_file())? || exists(directory.settings_file())? {
                return Err(FolderRefusal::NotDemo(directory.path().to_owned()).into());
            }
            std::fs::create_dir_all(directory.path())
                .with_context(|| format!("cannot create {}", directory.path().display()))?;
            Marker::write(&marker, scenario, false)?;
            Ok(Claimed::Fresh)
        }
        Some(kept) if kept.scenario != slug(scenario) => Err(FolderRefusal::OtherScenario {
            directory: directory.path().to_owned(),
            kept: kept.scenario,
            requested: scenario,
        }
        .into()),
        Some(kept) => {
            if kept.seeded {
                if exists(directory.database_file())? {
                    return Ok(Claimed::Kept);
                }
                Marker::write(&marker, scenario, false)?;
            }
            for suffix in ["", "-wal", "-shm"] {
                let mut path = directory.database_file().into_os_string();
                path.push(suffix);
                let path = PathBuf::from(path);
                match std::fs::remove_file(&path) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                        return Err(error)
                            .with_context(|| format!("cannot remove {}", path.display()));
                    }
                    _ => {}
                }
            }
            Ok(Claimed::Fresh)
        }
    }
}

/// One demo day: its ledger, its clock and source, and the last refresh.
pub struct DemoHost {
    connection: Connection,
    environment: DemoEnvironment<FixedTimeZoneProvider>,
    zones: FixedTimeZoneProvider,
    settings: WorkTimeSettings,
    store: SettingsStore,
    state: Option<WorkdayRefreshState>,
    /// The corrections of the week recorded so far, see [`CorrectionClock`].
    corrections: Cell<i64>,
    gains: ledger::GainsCache,
}

/// The demo's clock never moves, but a correction must come strictly after the one it withdraws:
/// each correction is recorded one microsecond after the previous one. The displayed times keep
/// their minutes and seconds.
struct CorrectionClock<'a> {
    clock: &'a dyn Clock,
    offset: SignedDuration,
}

impl Clock for CorrectionClock<'_> {
    fn now(&self) -> Timestamp {
        self.clock.now() + self.offset
    }
}

/// The demo runs in UTC: its scenarios fix local times, so any zone shows the same day.
fn zones() -> FixedTimeZoneProvider {
    FixedTimeZoneProvider::new("UTC", TimeZone::UTC)
}

impl DemoHost {
    /// Opens the demo of `scenario` kept in `directory`: its settings and its ledger, which a new
    /// folder, or a kept one whose ledger was deleted, seeds with the scenario's history. A folder
    /// with other WorkTime data, or with the demo of another scenario, is refused with a
    /// [`FolderRefusal`].
    pub fn open(scenario: DemoScenario, directory: &Path) -> anyhow::Result<Self> {
        let directory = DataDirectory::from(directory.to_owned());
        let claimed = claim(&directory, scenario)?;
        let path = directory.database_file();
        let connection = worktime_store::database::open(&path)
            .with_context(|| format!("cannot open the demo ledger {}", path.display()))?;
        worktime_store::database::migrate(&connection)?;
        let store = SettingsStore::new(directory.settings_file())?;
        let settings = store
            .load()
            .with_context(|| format!("cannot read {}", store.path().display()))?
            .settings;
        let environment = DemoEnvironment::new(zones(), scenario);
        // A kept ledger holds the corrections of earlier launches, at the same frozen time.
        let latest = ManualAdjustmentRepository::new(&connection)
            .latest_created_at()
            .map_err(|error| anyhow::anyhow!("cannot read the demo corrections: {error}"))?;
        let corrections = latest.map_or(0, |latest| {
            latest.duration_since(environment.now()).as_micros().max(0) as i64
        });
        let host = Self {
            connection,
            environment,
            zones: zones(),
            settings: WorkTimeSettings {
                // As the onboarding of a new install selects it.
                activity_source: settings
                    .activity_source
                    .or(Some(ActivitySourceKind::BuiltIn)),
                ..settings
            },
            store,
            state: None,
            corrections: Cell::new(corrections),
            gains: ledger::GainsCache::default(),
        };
        if claimed == Claimed::Kept {
            return Ok(host);
        }
        let read_settings = || host.settings.clone();
        let accounting = host.accounting(&read_settings);
        let corrections = ManualAdjustmentCommands::new(
            &accounting,
            ManualAdjustmentRepository::new(&host.connection),
        );
        host.environment.seed(
            &SessionEventRepository::new(&host.connection, &host.environment),
            &corrections,
        )?;
        Marker::write(&directory.path().join(MARKER), scenario, true)?;
        Ok(host)
    }

    pub fn settings(&self) -> &WorkTimeSettings {
        &self.settings
    }

    /// Writes `settings` to the demo's `settings.json` and works on them from now on.
    pub fn save_settings(&mut self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        self.store.save(settings)?;
        self.settings = settings.clone();
        Ok(())
    }

    fn accounting<'a>(
        &'a self,
        settings: &'a dyn Fn() -> WorkTimeSettings,
    ) -> WorkdayAccounting<'a> {
        WorkdayAccounting::new(
            &self.environment,
            SessionEventRepository::new(&self.connection, &self.environment),
            ManualAdjustmentRepository::new(&self.connection),
            &self.environment,
            &self.zones,
            settings,
        )
    }

    /// Recalculates the day as the app's refresh does and presents it in `language`.
    pub fn refresh(&mut self, language: Language) -> anyhow::Result<DayView> {
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let accounting = self.accounting(&read_settings);
        let store = WeeklyCreditRepository::new(&self.connection);
        let weekly = WeeklyRecalculation::new(&accounting, &store);
        let state = self
            .environment
            .refresh(&accounting, &weekly, self.state.as_ref())
            .map_err(|error| anyhow::anyhow!("cannot refresh the demo day: {error}"))?;
        let view = DayView::present(&state, &self.settings, &TimeZone::UTC, language);
        self.state = Some(state);
        Ok(view)
    }
}

impl DemoHost {
    /// The demo's week as its last refresh stored it; empty for a scenario of one day.
    pub fn week(&self) -> anyhow::Result<WeekTimeline> {
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let accounting = self.accounting(&read_settings);
        ledger::read_week(
            &self.connection,
            &accounting,
            &self.environment,
            &self.zones,
            &self.gains,
        )
    }

    /// Records a correction of the demo's week in its throwaway ledger.
    pub fn run_week_command(&self, command: &WeekCommand) -> Result<Vec<AdjustmentId>, Key> {
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let accounting = self.accounting(&read_settings);
        self.corrections.set(self.corrections.get() + 1);
        let clock = CorrectionClock {
            clock: &self.environment,
            offset: SignedDuration::from_micros(self.corrections.get()),
        };
        ledger::run(command, &self.connection, &accounting, &clock).map_err(|refusal| {
            refusal.logged(|error| log::error!("cannot correct the demo week: {error}"))
        })
    }
}

impl DemoHost {
    /// Records a correction of « Corriger le temps… » in the demo's throwaway ledger.
    pub fn save_adjustment(&self, adjustment: &ManualAdjustment) -> Result<AdjustmentId, Key> {
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let accounting = self.accounting(&read_settings);
        ledger::save(adjustment, &self.connection, &accounting).map_err(|refusal| {
            refusal.logged(|error| log::error!("cannot save the demo correction: {error}"))
        })
    }

    /// The correction `id` of `workday` in the demo's ledger, unless a later one replaced it.
    pub fn active_adjustment(
        &self,
        workday: &LocalWorkday,
        id: &AdjustmentId,
    ) -> Result<Option<ManualAdjustment>, Key> {
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let accounting = self.accounting(&read_settings);
        ledger::active_adjustment(workday, id, &self.connection, &accounting).map_err(|refusal| {
            refusal.logged(|error| log::error!("cannot read the demo correction: {error}"))
        })
    }

    /// The time a new correction is recorded at, after every correction before it: see
    /// [`CorrectionClock`].
    pub fn correction_time(&self) -> Timestamp {
        self.corrections.set(self.corrections.get() + 1);
        self.environment.now() + SignedDuration::from_micros(self.corrections.get())
    }

    pub fn zones(&self) -> &FixedTimeZoneProvider {
        &self.zones
    }

    pub fn now(&self) -> Timestamp {
        self.environment.now()
    }

    pub fn refresh_state(&self) -> Option<&WorkdayRefreshState> {
        self.state.as_ref()
    }

    /// The demo's local date.
    pub fn today(&self) -> jiff::civil::Date {
        self.environment.now().to_zoned(TimeZone::UTC).date()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip() {
        for scenario in DemoScenario::ALL {
            assert_eq!(from_slug(slug(scenario)), Some(scenario));
        }
        assert_eq!(from_slug("sunny"), None);
    }

    #[test]
    fn throwaway_folders_are_new_each_time() {
        let first = throwaway_directory();
        assert!(first.starts_with(std::env::temp_dir().join("WorkTimeDemo")));
        assert_ne!(first, throwaway_directory());
    }

    fn unrecorded(host: &mut DemoHost) -> Vec<worktime_domain::intervals::UtcInterval> {
        match host.refresh(Language::French).unwrap().source_hint {
            Some(crate::workday::SourceHint::Unrecorded { intervals, .. }) => intervals,
            other => panic!("not unrecorded time: {other:?}"),
        }
    }

    /// Quickstart 002 restarts a demo on `--data-dir` and finds its schedule and corrections.
    #[test]
    fn a_demo_reopened_on_its_folder_keeps_its_settings_and_corrections() {
        let directory = throwaway_directory();
        let mut host = DemoHost::open(DemoScenario::ProblemPeriods, &directory).unwrap();
        let settings = WorkTimeSettings {
            nominal_daily_target: SignedDuration::from_mins(7 * 60 + 24),
            ..host.settings().clone()
        };
        host.save_settings(&settings).unwrap();
        let intervals = unrecorded(&mut host);
        let workday = LocalWorkday::new(host.today(), "UTC");
        let declared = ManualAdjustment::interval_work(
            AdjustmentId::new("declared").unwrap(),
            workday.clone(),
            intervals[0],
            host.correction_time(),
        )
        .unwrap()
        .with_reason("Call");
        let id = host.save_adjustment(&declared).unwrap();
        drop(host);

        let mut host = DemoHost::open(DemoScenario::ProblemPeriods, &directory).unwrap();
        assert_eq!(host.settings(), &settings);
        assert_eq!(unrecorded(&mut host), intervals[1..]);
        assert_eq!(
            host.active_adjustment(&workday, &id).unwrap(),
            Some(declared)
        );
        let withdrawn = ManualAdjustment::revocation(
            AdjustmentId::new("withdrawn").unwrap(),
            workday,
            id,
            host.correction_time(),
        )
        .unwrap();
        host.save_adjustment(&withdrawn).unwrap();
        assert_eq!(unrecorded(&mut host), intervals);
        drop(host);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    fn refusal(scenario: DemoScenario, directory: &Path) -> FolderRefusal {
        match DemoHost::open(scenario, directory) {
            Ok(_) => panic!("the demo opened {}", directory.display()),
            Err(error) => error.downcast().unwrap(),
        }
    }

    fn contents(directory: &Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), std::fs::read(entry.path()).unwrap())
            })
            .collect();
        files.sort();
        files
    }

    /// `--demo --data-dir %LOCALAPPDATA%\WorkTime` leaves the user's ledger and settings alone.
    #[test]
    fn a_folder_of_real_data_is_refused_untouched() {
        let folder = tempfile::tempdir().unwrap();
        let directory = DataDirectory::from(folder.path().to_owned());
        let ledger = worktime_store::database::open(&directory.database_file()).unwrap();
        worktime_store::database::migrate(&ledger).unwrap();
        drop(ledger);
        SettingsStore::new(directory.settings_file())
            .unwrap()
            .save(&WorkTimeSettings::default())
            .unwrap();
        let before = contents(folder.path());

        let refused = refusal(DemoScenario::Late, folder.path());
        assert!(matches!(&refused, FolderRefusal::NotDemo(path) if path == folder.path()));
        assert_eq!(refused.message(), Key::StartupFailureDemoDataMessage);
        assert_eq!(contents(folder.path()), before);

        let settings_only = tempfile::tempdir().unwrap();
        SettingsStore::new(DataDirectory::from(settings_only.path().to_owned()).settings_file())
            .unwrap()
            .save(&WorkTimeSettings::default())
            .unwrap();
        assert!(matches!(
            refusal(DemoScenario::Late, settings_only.path()),
            FolderRefusal::NotDemo(_)
        ));
    }

    /// The corrections kept in a folder were recorded at its scenario's frozen time.
    #[test]
    fn a_kept_demo_refuses_another_scenario() {
        let folder = tempfile::tempdir().unwrap();
        drop(DemoHost::open(DemoScenario::ProblemPeriods, folder.path()).unwrap());
        let before = contents(folder.path());

        let refused = refusal(DemoScenario::Late, folder.path());
        assert!(matches!(
            &refused,
            FolderRefusal::OtherScenario { kept, requested: DemoScenario::Late, .. }
                if kept == "problem-periods"
        ));
        assert_eq!(refused.message(), Key::StartupFailureDemoScenarioMessage);
        assert_eq!(contents(folder.path()), before);
        assert!(DemoHost::open(DemoScenario::ProblemPeriods, folder.path()).is_ok());
    }

    /// A first launch that stopped after its marker, or during its seed, or before it marked the
    /// seed done, is seeded again on the next one, as once.
    #[test]
    fn a_seed_that_did_not_finish_is_seeded_again() {
        let scenario = DemoScenario::HistoryWeek;
        let day_and_week = |folder: &Path| {
            let mut host = DemoHost::open(scenario, folder).unwrap();
            let day = host.refresh(Language::English).unwrap();
            (day, host.week().unwrap())
        };
        let unseeded = |folder: &Path| {
            Marker::write(&folder.join(MARKER), scenario, false).unwrap();
        };
        let fresh = tempfile::tempdir().unwrap();
        let expected = day_and_week(fresh.path());
        assert!(!expected.1.days.is_empty());

        let marked_only = tempfile::tempdir().unwrap();
        unseeded(marked_only.path());
        let empty_ledger = tempfile::tempdir().unwrap();
        unseeded(empty_ledger.path());
        let ledger = worktime_store::database::open(
            &DataDirectory::from(empty_ledger.path().to_owned()).database_file(),
        )
        .unwrap();
        worktime_store::database::migrate(&ledger).unwrap();
        drop(ledger);
        let seeded_unmarked = tempfile::tempdir().unwrap();
        drop(DemoHost::open(scenario, seeded_unmarked.path()).unwrap());
        unseeded(seeded_unmarked.path());

        for folder in [&marked_only, &empty_ledger, &seeded_unmarked] {
            assert_eq!(day_and_week(folder.path()), expected);
            assert_eq!(day_and_week(folder.path()), expected);
        }
    }

    /// A kept demo whose `worktime.db` was deleted by hand shows its scenario again, not an empty
    /// day, and keeps its settings.
    #[test]
    fn a_kept_demo_that_lost_its_ledger_is_seeded_again() {
        let scenario = DemoScenario::HistoryWeek;
        let with_settings = |folder: &Path| {
            let mut host = DemoHost::open(scenario, folder).unwrap();
            let settings = WorkTimeSettings {
                nominal_daily_target: SignedDuration::from_mins(7 * 60 + 24),
                ..host.settings().clone()
            };
            host.save_settings(&settings).unwrap();
            (host, settings)
        };
        let day_and_week = |host: &mut DemoHost| {
            let day = host.refresh(Language::English).unwrap();
            (day, host.week().unwrap())
        };
        let fresh = tempfile::tempdir().unwrap();
        let (mut host, settings) = with_settings(fresh.path());
        let expected = day_and_week(&mut host);
        assert!(!expected.1.days.is_empty());
        drop(host);

        let kept = tempfile::tempdir().unwrap();
        drop(with_settings(kept.path()));
        std::fs::remove_file(DataDirectory::from(kept.path().to_owned()).database_file()).unwrap();

        for _ in 0..2 {
            let mut host = DemoHost::open(scenario, kept.path()).unwrap();
            assert_eq!(host.settings(), &settings);
            assert_eq!(day_and_week(&mut host), expected);
        }
    }
}
