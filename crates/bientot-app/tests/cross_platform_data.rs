//! A `.bientot` file exported on one platform imports on the other with the same week
//! (`specs/005-linux-support`, SC-010). The two exports in `bientot-store/tests/fixtures` hold
//! the same week, worked in Paris, as each platform writes it: the zone named by its Windows key
//! or by its IANA name, and the power history read from the System log or from the journal.
//! `write_the_fixtures` writes them again.

use std::path::PathBuf;

use bientot_app::demo;
use bientot_app::live::{self, LiveHost};
use bientot_app::week::{Availability, SegmentKind, WeekTimeline};
use bientot_app::zones::SystemZones;
use bientot_domain::sources::{ActivitySourceKind, ActivitySourceSelection, IdleSpan, IdleState};
use bientot_domain::time::{
    Clock as _, FixedTimeZoneProvider, FrozenClock, LocalWorkday, TimeZoneProvider,
    UnknownTimeZone, WorkdayCalendar,
};
use bientot_platform::power_history::{
    PowerBoundary, PowerBoundaryKind, PowerHistoryRead, PowerHistoryRecovery, pair,
};
use bientot_store::activity::{ActivitySourceSelectionRepository, ActivitySpanRepository};
use bientot_store::data_directory::DataDirectory;
use bientot_store::sessions::SessionEventRepository;
use bientot_store::transfer;
use jiff::Timestamp;
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;

/// Friday 9 October 2026 at 17:30 in Paris, when the week was exported, its credit not yet
/// expired.
const EXPORTED: &str = "2026-10-09T15:30:00Z";
/// Half an hour later, when the export is imported.
const IMPORTED: &str = "2026-10-09T16:00:00Z";
/// The next Monday at 9:00 in Paris, when the week is closed.
const NEXT_MONDAY: &str = "2026-10-12T07:00:00Z";

#[derive(Clone, Copy)]
enum Platform {
    Windows,
    Linux,
}

impl Platform {
    /// How the platform names Paris in the ledger.
    fn zone_id(self) -> &'static str {
        match self {
            Self::Windows => "Romance Standard Time",
            Self::Linux => "Europe/Paris",
        }
    }

    fn fixture(self) -> PathBuf {
        let name = match self {
            Self::Windows => "export-windows.bientot",
            Self::Linux => "export-linux.bientot",
        };
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("bientot-store")
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    /// The source keys of a suspend and a resume, as each platform's log identifies them.
    fn power_source_keys(self) -> [&'static str; 2] {
        match self {
            Self::Windows => [
                "microsoft-windows-kernel-power:42:51207",
                "microsoft-windows-kernel-power:107:51208",
            ],
            // `_BOOT_ID` and `__CURSOR` (contracts/linux-backends.md); the journal reader (T092)
            // may still change how it joins them.
            Self::Linux => [
                "3f2a9c1be8d54a7f9e0c6b5d4a3f2e1d:s=8c1d0e2f;i=1b2c3;b=3f2a9c1be8d54a7f9e0c6b5d4a3f2e1d;m=2a3b4c5d;t=6421f0e1d2c3b;x=9d8c7b6a",
                "3f2a9c1be8d54a7f9e0c6b5d4a3f2e1d:s=8c1d0e2f;i=1b2c4;b=3f2a9c1be8d54a7f9e0c6b5d4a3f2e1d;m=2a3b4c5e;t=6421f0e1d2c3c;x=9d8c7b6b",
            ],
        }
    }
}

fn paris() -> TimeZone {
    jiff::tz::db().get("Europe/Paris").unwrap()
}

fn monday() -> Date {
    date(2026, 10, 5)
}

/// `hour`:`minute` in Paris on the day `day` days after Monday 5 October 2026.
fn in_paris(day: i64, hour: i8, minute: i8) -> Timestamp {
    monday()
        .checked_add(jiff::Span::new().days(day))
        .unwrap()
        .at(hour, minute, 0, 0)
        .to_zoned_datetime(paris())
        .unwrap()
        .timestamp()
}

/// The system's zones on a Linux computer set to Paris, whatever the zone of the one running the
/// test.
struct LinuxInParis;

impl TimeZoneProvider for LinuxInParis {
    fn local(&self) -> (String, TimeZone) {
        ("Europe/Paris".to_owned(), paris())
    }

    fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
        if id == "Europe/Paris" {
            return Ok(paris());
        }
        SystemZones.get(id)
    }
}

/// The export of `platform` imported into a new data folder, as the next start applies it.
fn imported(platform: Platform) -> (tempfile::TempDir, DataDirectory) {
    let temporary = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(temporary.path().join("data"));
    transfer::stage(&platform.fixture(), &directory).unwrap();
    let applied = transfer::apply_staged(&directory, IMPORTED.parse().unwrap(), &paris());
    assert!(applied.unwrap().is_some());
    (temporary, directory)
}

/// A start on the export of `platform` imported, at `now` on a Linux computer in Paris: whether
/// its refresh failed, and the week it shows.
fn week_shown(platform: Platform, now: &str) -> (bool, WeekTimeline) {
    let (_temporary, directory) = imported(platform);
    let loaded = live::load_settings(&directory).unwrap();
    let connection = live::open_ledger(&directory).unwrap();
    let clock = FrozenClock::new(now.parse().unwrap());
    let mut host = LiveHost::start(
        &directory,
        loaded,
        &connection,
        &clock,
        &LinuxInParis,
        &demo::Typing,
    )
    .unwrap();
    host.refresh();
    let shown = (host.refresh_failed(), host.week().unwrap());
    host.shut_down();
    shown
}

#[test]
fn a_windows_export_goes_on_into_the_next_week_on_linux() {
    let (failed, week) = week_shown(Platform::Windows, NEXT_MONDAY);

    assert!(!failed, "the week of the export replays in its own zone");
    let monday = week.days.first().unwrap();
    assert_eq!(monday.date, date(2026, 10, 12));
    assert_eq!(monday.availability, Availability::Loaded);
}

#[test]
fn exports_from_either_platform_show_the_same_week() {
    for now in [IMPORTED, NEXT_MONDAY] {
        let from_windows = week_shown(Platform::Windows, now);
        let from_linux = week_shown(Platform::Linux, now);

        assert_eq!(from_windows, from_linux, "at {now}");
    }
    let (_, week) = week_shown(Platform::Linux, IMPORTED);
    let wednesday = &week.days[2];
    assert!(
        wednesday
            .segments
            .iter()
            .any(|segment| segment.kind == SegmentKind::Sleep),
        "the suspend the journal logged shows: {wednesday:#?}"
    );
}

#[test]
#[ignore = "writes the fixtures again: run it after a change to what the ledger stores"]
fn write_the_fixtures() {
    for platform in [Platform::Windows, Platform::Linux] {
        write_fixture(platform);
    }
}

/// Records Monday to Friday, worked 9:00 to 12:30 and 13:30 to 17:00 in Paris, Tuesday to 18:00
/// for an hour of credit, and Wednesday's lunch asleep, as `platform` records it, then exports it
/// on Friday evening.
fn write_fixture(platform: Platform) {
    let temporary = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(temporary.path().to_owned());
    let zones = FixedTimeZoneProvider::new(platform.zone_id(), paris());
    let clock = FrozenClock::new(EXPORTED.parse().unwrap());
    let connection = live::open_ledger(&directory).unwrap();
    bientot_store::database::migrate(&connection).unwrap();

    ActivitySourceSelectionRepository::new(&connection, &clock)
        .append(ActivitySourceSelection {
            effective_from: in_paris(0, 0, 0),
            source: ActivitySourceKind::BuiltIn,
        })
        .unwrap();
    let spans = ActivitySpanRepository::new(&connection, &clock);
    let mut idle_from = in_paris(0, 0, 0);
    for day in 0..5 {
        let end = if day == 1 { 18 } else { 17 };
        for (state, start, end) in [
            (IdleState::Idle, idle_from, in_paris(day, 9, 0)),
            (
                IdleState::Active,
                in_paris(day, 9, 0),
                in_paris(day, 12, 30),
            ),
            (
                IdleState::Idle,
                in_paris(day, 12, 30),
                in_paris(day, 13, 30),
            ),
            (
                IdleState::Active,
                in_paris(day, 13, 30),
                in_paris(day, end, 0),
            ),
        ] {
            spans
                .append(IdleSpan::new(state, start, end).unwrap(), None)
                .unwrap();
        }
        idle_from = in_paris(day, end, 0);
    }
    spans
        .append(
            IdleSpan::new(IdleState::Idle, idle_from, clock.now()).unwrap(),
            None,
        )
        .unwrap();

    let wednesday = WorkdayCalendar::new(&zones)
        .window(&LocalWorkday::new(date(2026, 10, 7), platform.zone_id()))
        .unwrap();
    let [suspend, resume] = platform.power_source_keys();
    let history = pair([
        PowerBoundary {
            kind: PowerBoundaryKind::Suspend,
            at: in_paris(2, 12, 35),
            source_key: suspend.to_owned(),
        },
        PowerBoundary {
            kind: PowerBoundaryKind::Resume,
            at: in_paris(2, 13, 25),
            source_key: resume.to_owned(),
        },
    ]);
    PowerHistoryRecovery::new(
        SessionEventRepository::new(&connection, &clock),
        PowerHistoryRecovery::READ_TIMEOUT,
    )
    .recover(&wednesday, move |_| PowerHistoryRead::Complete(history))
    .unwrap();

    let mut loaded = live::load_settings(&directory).unwrap();
    loaded.settings.activity_source = Some(ActivitySourceKind::BuiltIn);
    let settings = loaded.settings.clone();
    let mut host = LiveHost::start(
        &directory,
        loaded,
        &connection,
        &clock,
        &zones,
        &demo::Typing,
    )
    .unwrap();
    host.save_settings(&settings).unwrap();
    host.poll();
    host.refresh();
    host.shut_down();
    drop(host);

    transfer::export(&directory, &platform.fixture(), clock.now()).unwrap();
}
