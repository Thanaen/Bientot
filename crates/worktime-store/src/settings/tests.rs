use std::fs;

use jiff::civil::{Time, Weekday};
use jiff::{SignedDuration, Timestamp};
use serde_json::Value;
use tempfile::TempDir;
use worktime_domain::configuration::{
    ActivityWatchSettings, ActivityWatchSettlementContract, BuiltInActivitySettings,
    LoggingSettings, NotificationRule, NotificationSettings, RecurringAppointment, UpdateSettings,
};
use worktime_domain::sources::ActivitySourceKind;

use super::*;

struct Fixture {
    _directory: TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("settings.json");
        Self {
            _directory: directory,
            path,
        }
    }

    fn with_file(contents: &str) -> Self {
        let fixture = Self::new();
        fs::write(&fixture.path, contents).unwrap();
        fixture
    }

    fn store(&self) -> SettingsStore {
        SettingsStore::new(&self.path).unwrap()
    }

    fn load(&self) -> LoadedSettings {
        self.store().load().unwrap()
    }

    fn contents(&self) -> String {
        fs::read_to_string(&self.path).unwrap()
    }

    fn file_names(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(self.path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

/// The text .NET writes, with this platform's line breaks.
fn as_written(dotnet_text: &str) -> String {
    dotnet_text.replace('\n', NEWLINE)
}

/// `dotnet_text` with the `usagePing`, empty `recurringAppointments` and default
/// `appointmentProposals` members .NET does not have, where WorkTime writes them.
/// `appointmentProposals` as WorkTime writes it by default, after a line break.
const PROPOSALS: &str =
    "\n  \"appointmentProposals\": {\n    \"enabled\": true,\n    \"dismissed\": []\n  },";

fn with_rust_only_members(dotnet_text: &str, usage_ping: bool) -> String {
    let logging = "\n  \"logging\": {";
    assert_eq!(dotnet_text.matches(logging).count(), 1, "{dotnet_text}");
    dotnet_text.replace(
        logging,
        &format!(
            "\n  \"usagePing\": {usage_ping},\n  \"recurringAppointments\": [],{PROPOSALS}{logging}"
        ),
    )
}

#[test]
fn round_trips_every_notification_preference() {
    let fixture = Fixture::new();
    let candidate = WorkTimeSettings {
        notifications: NotificationSettings {
            completion_approach: NotificationRule::new(false, minutes(7)),
            latest_departure_approach: NotificationRule::new(true, minutes(23)),
            day_complete: NotificationRule::new(true, SignedDuration::ZERO),
        },
        ..WorkTimeSettings::default()
    };

    fixture.store().save(&candidate).unwrap();

    assert_eq!(
        fixture.load().settings.notifications,
        candidate.notifications
    );
}

#[test]
fn invalid_settings_are_preserved_and_recovered() {
    for json in [
        "{broken",
        "null",
        r#"{"nominalDailyTarget":"00:00:00"}"#,
        r#"{"activityWatch":null}"#,
    ] {
        let fixture = Fixture::with_file(json);
        let store = fixture.store();

        let loaded = store.load().unwrap();

        assert_eq!(loaded.settings, WorkTimeSettings::default(), "{json}");
        let SettingsSource::Recovered { backup } = loaded.source else {
            panic!("{json} was not recovered: {:?}", loaded.source);
        };
        assert_eq!(fs::read_to_string(&backup).unwrap(), json);
        assert!(!fixture.path.exists(), "{json}");
        store.save(&WorkTimeSettings::default()).unwrap();
        assert_eq!(fixture.load().settings, WorkTimeSettings::default());
    }
}

#[test]
fn peeking_at_invalid_settings_leaves_the_file_in_place() {
    let fixture = Fixture::with_file("{broken");
    let store = fixture.store();

    assert_eq!(store.peek(), None);
    assert_eq!(fs::read_to_string(&fixture.path).unwrap(), "{broken");
    assert_eq!(
        fs::read_dir(fixture.path.parent().unwrap())
            .unwrap()
            .count(),
        1,
        "nothing moved aside"
    );
}

#[test]
fn peeking_reads_valid_settings() {
    let fixture = Fixture::new();
    let settings = WorkTimeSettings {
        language_override: Some("en-US".to_owned()),
        ..WorkTimeSettings::default()
    };
    fixture.store().save(&settings).unwrap();

    assert_eq!(fixture.store().peek(), Some(settings));
}

#[test]
fn peeking_without_a_file_reads_the_defaults_and_writes_nothing() {
    let fixture = Fixture::new();

    assert_eq!(fixture.store().peek(), Some(WorkTimeSettings::default()));
    assert!(!fixture.path.exists(), "no file written");
}

#[test]
fn peeking_at_a_newer_schema_reads_nothing_and_leaves_it_alone() {
    let json = r#"{"schemaVersion":99}"#;
    let fixture = Fixture::with_file(json);

    assert_eq!(fixture.store().peek(), None);
    assert_eq!(fixture.contents(), json);
}

#[test]
fn version_zero_and_missing_fields_migrate_to_current_defaults() {
    let fixture = Fixture::with_file(r#"{"schemaVersion":0,"nominalDailyTarget":"08:00:00"}"#);
    let store = fixture.store();

    let settings = store.load().unwrap().settings;

    assert_eq!(settings.schema_version, 1);
    assert_eq!(settings.nominal_daily_target, SignedDuration::from_hours(8));
    assert_eq!(settings.lunch_start, Time::constant(12, 0, 0, 0));
    store.save(&settings).unwrap();
    assert_eq!(fixture.load().settings, settings);
}

#[test]
fn debt_settings_round_trip_and_older_files_load_with_debt_off() {
    let fixture = Fixture::with_file(r#"{"schemaVersion":1,"weeklyCreditEnabled":true}"#);
    let store = fixture.store();
    let legacy = store.load().unwrap().settings;
    assert!(!legacy.weekly_debt_enabled);
    assert_eq!(legacy.maximum_weekly_debt, None);

    let candidate = WorkTimeSettings {
        weekly_debt_enabled: true,
        maximum_weekly_debt: Some(minutes(95)),
        ..legacy
    };
    store.save(&candidate).unwrap();

    assert_eq!(fixture.load().settings, candidate);
}

#[test]
fn activity_source_and_idle_threshold_round_trip_with_the_ledger_spelling() {
    for (source, stored_source) in [
        (
            ActivitySourceKind::BuiltIn,
            r#""activitySource": "built-in""#,
        ),
        (
            ActivitySourceKind::ActivityWatch,
            r#""activitySource": "activitywatch""#,
        ),
    ] {
        let fixture = Fixture::new();
        let candidate = WorkTimeSettings {
            activity_source: Some(source),
            built_in_activity: BuiltInActivitySettings {
                idle_threshold: minutes(5),
            },
            ..WorkTimeSettings::default()
        };

        fixture.store().save(&candidate).unwrap();

        assert!(fixture.contents().contains(stored_source), "{source:?}");
        assert_eq!(fixture.load().settings, candidate);
    }
}

#[test]
fn a_settings_file_written_before_the_built_in_source_has_no_activity_source_and_the_default_threshold()
 {
    let fixture = Fixture::with_file(
        r#"{"schemaVersion":1,"activityWatch":{"baseUrl":"http://127.0.0.1:5600"}}"#,
    );

    let settings = fixture.load().settings;

    assert_eq!(settings.activity_source, None);
    assert_eq!(
        settings.built_in_activity.idle_threshold,
        SignedDuration::from_secs(180)
    );
}

#[test]
fn an_unknown_activity_source_or_threshold_is_preserved_and_recovered() {
    for json in [
        r#"{"activitySource":"aw"}"#,
        r#"{"activitySource":0}"#,
        r#"{"builtInActivity":{"idleThreshold":"00:00:10"}}"#,
        r#"{"builtInActivity":null}"#,
    ] {
        let fixture = Fixture::with_file(json);

        let loaded = fixture.load();

        assert_eq!(loaded.settings, WorkTimeSettings::default(), "{json}");
        let SettingsSource::Recovered { backup } = loaded.source else {
            panic!("{json} was not recovered: {:?}", loaded.source);
        };
        assert_eq!(fs::read_to_string(backup).unwrap(), json);
    }
}

#[test]
fn future_schema_is_never_overwritten() {
    let json = r#"{"schemaVersion":99}"#;
    let fixture = Fixture::with_file(json);

    let error = fixture.store().load().unwrap_err();

    assert!(
        matches!(error, LoadSettingsError::NewerSchema { schema_version: 99 }),
        "{error:?}"
    );
    assert_eq!(fixture.contents(), json);
}

#[test]
fn invalid_save_leaves_existing_settings_intact() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.save(&WorkTimeSettings::default()).unwrap();

    let error = store
        .save(&WorkTimeSettings {
            lunch_end: Time::constant(10, 0, 0, 0),
            ..WorkTimeSettings::default()
        })
        .unwrap_err();

    assert!(
        matches!(&error, SaveSettingsError::Invalid(errors) if errors == &[SettingsError::Lunch]),
        "{error:?}"
    );
    assert_eq!(store.load().unwrap().settings, WorkTimeSettings::default());
}

#[test]
fn a_missing_file_loads_the_defaults_and_says_so() {
    let fixture = Fixture::new();

    assert_eq!(
        fixture.load(),
        LoadedSettings {
            settings: WorkTimeSettings::default(),
            source: SettingsSource::Missing,
        }
    );
    assert_eq!(fixture.file_names(), Vec::<String>::new());
}

#[test]
fn a_save_creates_the_directory_and_leaves_only_the_settings_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("WorkTime").join("settings.json");
    let store = SettingsStore::new(&path).unwrap();

    store.save(&WorkTimeSettings::default()).unwrap();

    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["settings.json"]);
    assert_eq!(
        store.load().unwrap().source,
        SettingsSource::File,
        "{}",
        fs::read_to_string(&path).unwrap()
    );
}

#[test]
fn a_failed_save_removes_its_temporary_file() {
    let fixture = Fixture::new();
    // A directory with content cannot be replaced by a file, so the final rename fails.
    fs::create_dir_all(fixture.path.join("occupied")).unwrap();

    let error = fixture
        .store()
        .save(&WorkTimeSettings::default())
        .unwrap_err();

    assert!(matches!(error, SaveSettingsError::Io(_)), "{error:?}");
    assert_eq!(fixture.file_names(), ["settings.json"]);
    assert!(fixture.path.is_dir());
}

/// Replays `tests/fixtures/settings-json.json`. For each input it holds what the .NET deserializer
/// reads (written back by the .NET serializer, or null when it rejects the input), what
/// `JsonSettingsStore.LoadAsync` does with it, and the file `SaveAsync` then writes when that
/// differs from the re-serialized settings.
#[test]
fn settings_files_read_load_and_save_like_dotnet() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/settings-json.json")).unwrap();
    for case in cases {
        let input = case["input"].as_str().unwrap();
        if crate::codec::tests::beyond_timestamp_range(input) {
            assert!(parse(input.as_bytes()).is_err(), "{input:?}");
            continue;
        }
        let parsed = case["parsed"]
            .as_str()
            .map(|parsed| with_rust_only_members(parsed, true));

        let read = parse(input.as_bytes()).map(|settings| json::write(&settings, "\n", None));
        assert_eq!(
            read.as_ref().ok(),
            parsed.as_ref(),
            "read {input:?}: {read:?}"
        );

        let fixture = Fixture::with_file(input);
        let store = fixture.store();
        match (store.load(), case["load"].as_str().unwrap()) {
            (Ok(loaded), "loaded") => {
                assert_eq!(loaded.source, SettingsSource::File, "{input:?}");
                store.save(&loaded.settings).unwrap();
                let saved = case["saved"]
                    .as_str()
                    .map(|saved| with_rust_only_members(saved, true))
                    .or(parsed)
                    .unwrap();
                // .NET drops the members it does not know; the store keeps them after the rest.
                let contents = fixture.contents();
                assert_eq!(
                    json::write(&loaded.settings, NEWLINE, None),
                    as_written(&saved),
                    "saved {input:?}"
                );
                assert_eq!(
                    contents,
                    json::write(&loaded.settings, NEWLINE, Some(input)),
                    "saved {input:?}"
                );
                assert_eq!(parse(contents.as_bytes()), Ok(loaded.settings), "{input:?}");
            }
            (Ok(loaded), "recovered") => {
                let SettingsSource::Recovered { backup } = loaded.source else {
                    panic!("{input:?} was not recovered: {:?}", loaded.source);
                };
                assert_eq!(loaded.settings, WorkTimeSettings::default(), "{input:?}");
                assert_eq!(fs::read_to_string(backup).unwrap(), input);
                assert!(!fixture.path.exists(), "{input:?}");
            }
            (Err(LoadSettingsError::NewerSchema { .. }), "newer") => {
                assert_eq!(fixture.contents(), input);
            }
            (outcome, expected) => panic!("{input:?}: expected {expected}, got {outcome:?}"),
        }
    }
}

/// Mirrors `CustomSettings()` of the .NET fixture export that wrote `settings-custom.json`.
fn custom_settings() -> WorkTimeSettings {
    WorkTimeSettings {
        schema_version: 1,
        nominal_daily_target: SignedDuration::new(7 * 3600 + 42 * 60 + 30, 500_000_000),
        minimum_effective_target: SignedDuration::from_hours(5) + SignedDuration::from_nanos(100),
        lunch_start: Time::constant(11, 45, 10, 0),
        lunch_end: Time::constant(13, 15, 0, 123_456_700),
        preferred_latest_departure: Time::constant(19, 5, 0, 0),
        orange_warning_margin: SignedDuration::ZERO,
        long_lock_review_threshold: SignedDuration::from_hours(24),
        weekly_credit_enabled: false,
        weekly_debt_enabled: true,
        maximum_weekly_debt: Some(minutes(95)),
        start_with_windows: false,
        notifications: NotificationSettings {
            completion_approach: NotificationRule::new(false, minutes(7)),
            latest_departure_approach: NotificationRule::new(true, SignedDuration::from_nanos(100)),
            day_complete: NotificationRule::new(true, SignedDuration::from_hours(24)),
        },
        activity_source: Some(ActivitySourceKind::ActivityWatch),
        built_in_activity: BuiltInActivitySettings {
            idle_threshold: SignedDuration::from_secs(30),
        },
        activity_watch: ActivityWatchSettings {
            base_url: "https://aw.example:5601/".to_owned(),
            selected_bucket_id: Some(
                "aw-watcher-afk_h\u{f4}te <&> 'quote' \"double\" back\\slash +plus `tick` \u{7f}\u{1}\t\n\r\u{8}\u{c} \u{1F600} \u{2028} \u{a0} \u{feff}"
                    .to_owned(),
            ),
            api_key_reference: Some("activitywatch-token".to_owned()),
            request_timeout: SignedDuration::from_millis(1500),
            settlement: ActivityWatchSettlementContract {
                afk_timeout: minutes(20),
                poll_interval: SignedDuration::from_secs(1),
                maximum_jitter: SignedDuration::from_nanos(999_999_900),
            },
        },
        updates: UpdateSettings {
            check_for_updates: false,
            last_checked_at: Some("2026-10-13T08:15:30.12Z".parse::<Timestamp>().unwrap()),
        },
        usage_ping: true,
        recurring_appointments: Vec::new(),
        appointment_proposals: Default::default(),
        logging: LoggingSettings {
            maximum_file_count: 12,
            maximum_file_bytes: 20 * 1024 * 1024,
        },
        language_override: Some("fr-FR".to_owned()),
    }
}

#[test]
fn settings_are_written_byte_for_byte_as_dotnet_writes_them() {
    let dotnet = include_str!("../../tests/fixtures/settings-custom.json");
    let fixture = Fixture::new();
    let store = fixture.store();

    store.save(&custom_settings()).unwrap();

    assert_eq!(
        fixture.contents(),
        as_written(&with_rust_only_members(dotnet, true))
    );
    assert_eq!(
        store.load().unwrap(),
        LoadedSettings {
            settings: custom_settings(),
            source: SettingsSource::File,
        }
    );
}

#[test]
fn a_settings_file_written_by_dotnet_loads_to_the_same_values() {
    let fixture = Fixture::with_file(include_str!("../../tests/fixtures/settings-custom.json"));

    assert_eq!(fixture.load().settings, custom_settings());
}

#[test]
fn a_settings_file_without_the_usage_signal_setting_loads_with_the_signal_on() {
    let fixture = Fixture::with_file(r#"{"schemaVersion":1,"updates":{"checkForUpdates":false}}"#);

    let settings = fixture.load().settings;

    assert!(settings.usage_ping);
    assert!(!settings.updates.check_for_updates);
}

#[test]
fn a_turned_off_usage_signal_is_written_after_the_update_settings_and_read_back() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let off = WorkTimeSettings {
        usage_ping: false,
        ..custom_settings()
    };

    store.save(&off).unwrap();

    let dotnet = include_str!("../../tests/fixtures/settings-custom.json");
    assert_eq!(
        fixture.contents(),
        as_written(&with_rust_only_members(dotnet, false))
    );
    assert_eq!(store.load().unwrap().settings, off);
}

#[test]
fn the_usage_signal_setting_matches_in_any_case_and_rejects_a_non_boolean() {
    let fixture = Fixture::with_file(r#"{"UsagePing":false}"#);
    assert!(!fixture.load().settings.usage_ping);

    let fixture = Fixture::with_file(r#"{"usagePing":"false"}"#);
    let loaded = fixture.load();
    assert!(matches!(loaded.source, SettingsSource::Recovered { .. }));
    assert!(loaded.settings.usage_ping);
}

#[test]
fn a_save_keeps_the_members_it_does_not_know_after_the_known_ones() {
    let fixture = Fixture::with_file(
        r#"{"schemaVersion":1,"theme":"dark","notifications":{"completionApproach":{"enabled":false,"snooze":"00:05:00"},"weeklySummary":{"days":[1,2],"empty":{},"none":[]}},"note":"café","theme":"light"}"#,
    );
    let store = fixture.store();
    let mut settings = store.load().unwrap().settings;
    settings.weekly_credit_enabled = false;

    store.save(&settings).unwrap();

    let expected = r#"{
  "schemaVersion": 1,
  "nominalDailyTarget": "07:00:00",
  "minimumEffectiveTarget": "06:00:00",
  "lunchStart": "12:00:00",
  "lunchEnd": "14:00:00",
  "preferredLatestDeparture": "18:30:00",
  "orangeWarningMargin": "00:15:00",
  "longLockReviewThreshold": "01:30:00",
  "weeklyCreditEnabled": false,
  "weeklyDebtEnabled": false,
  "maximumWeeklyDebt": null,
  "startWithWindows": true,
  "notifications": {
    "completionApproach": {
      "enabled": false,
      "leadTime": "00:00:00",
      "snooze": "00:05:00"
    },
    "latestDepartureApproach": {
      "enabled": true,
      "leadTime": "00:15:00"
    },
    "dayComplete": {
      "enabled": false,
      "leadTime": "00:00:00"
    },
    "weeklySummary": {
      "days": [
        1,
        2
      ],
      "empty": {},
      "none": []
    }
  },
  "activitySource": null,
  "builtInActivity": {
    "idleThreshold": "00:03:00"
  },
  "activityWatch": {
    "baseUrl": "http://127.0.0.1:5600",
    "selectedBucketId": null,
    "apiKeyReference": null,
    "requestTimeout": "00:00:05",
    "settlement": {
      "afkTimeout": "00:03:00",
      "pollInterval": "00:00:05",
      "maximumJitter": "00:00:05",
      "horizon": "00:03:10"
    }
  },
  "updates": {
    "checkForUpdates": true,
    "lastCheckedAtUtc": null
  },
  "usagePing": true,
  "recurringAppointments": [],
  "appointmentProposals": {
    "enabled": true,
    "dismissed": []
  },
  "logging": {
    "maximumFileCount": 5,
    "maximumFileBytes": 1048576
  },
  "languageOverride": null,
  "theme": "light",
  "note": "café"
}"#;
    assert_eq!(fixture.contents(), as_written(expected));
    assert_eq!(store.load().unwrap().settings, settings);
}

#[test]
fn a_member_known_in_another_case_is_written_once_in_camel_case() {
    let fixture = Fixture::with_file(r#"{"WeeklyCreditEnabled":true,"Extra":1}"#);
    let store = fixture.store();
    let settings = WorkTimeSettings {
        weekly_credit_enabled: false,
        ..WorkTimeSettings::default()
    };

    store.save(&settings).unwrap();

    let contents = fixture.contents();
    assert_eq!(
        contents.matches("eeklyCreditEnabled").count(),
        1,
        "{contents}"
    );
    assert!(
        contents.contains("\"weeklyCreditEnabled\": false,"),
        "{contents}"
    );
    assert!(
        contents.ends_with(&as_written(
            "  \"languageOverride\": null,\n  \"Extra\": 1\n}"
        )),
        "{contents}"
    );
}

#[test]
fn an_unreadable_file_leaves_nothing_to_keep() {
    let fixture = Fixture::with_file("{not json");
    let store = fixture.store();

    store.save(&WorkTimeSettings::default()).unwrap();

    assert_eq!(
        fixture.contents(),
        json::write(&WorkTimeSettings::default(), NEWLINE, None)
    );
}

#[test]
fn recurring_appointments_are_written_after_the_usage_signal_and_read_back() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let settings = WorkTimeSettings {
        recurring_appointments: vec![RecurringAppointment {
            id: "thursday-meeting".to_owned(),
            name: "R\u{e9}union du jeudi".to_owned(),
            weekday: Weekday::Thursday,
            start: Time::constant(14, 0, 0, 0),
            end: Time::constant(16, 0, 0, 0),
            enabled: false,
            since: Some(jiff::civil::date(2026, 10, 23)),
            alternate_weeks_from: Some(jiff::civil::date(2026, 10, 29)),
            automatic: true,
        }],
        ..WorkTimeSettings::default()
    };

    store.save(&settings).unwrap();

    let expected = [
        "  \"recurringAppointments\": [",
        "    {",
        "      \"id\": \"thursday-meeting\",",
        "      \"name\": \"R\\u00E9union du jeudi\",",
        "      \"weekday\": \"thursday\",",
        "      \"start\": \"14:00:00\",",
        "      \"end\": \"16:00:00\",",
        "      \"enabled\": false,",
        "      \"since\": \"2026-10-23\",",
        "      \"alternateWeeksFrom\": \"2026-10-29\",",
        "      \"automatic\": true",
        "    }",
        "  ],",
    ]
    .join(NEWLINE);
    assert!(
        fixture.contents().contains(&expected),
        "{}",
        fixture.contents()
    );
    assert_eq!(store.load().unwrap().settings, settings);
}

#[test]
fn turned_down_proposals_are_written_after_the_appointments_and_read_back() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let settings = WorkTimeSettings {
        appointment_proposals: worktime_domain::configuration::AppointmentProposalSettings {
            enabled: false,
            dismissed: vec![
                worktime_domain::configuration::DismissedProposal {
                    weekday: Weekday::Thursday,
                    start: Time::constant(14, 0, 0, 0),
                    end: Time::constant(16, 0, 0, 0),
                    until: Some(jiff::civil::date(2026, 11, 5)),
                },
                worktime_domain::configuration::DismissedProposal {
                    weekday: Weekday::Monday,
                    start: Time::constant(9, 0, 0, 0),
                    end: Time::constant(9, 30, 0, 0),
                    until: None,
                },
            ],
        },
        ..WorkTimeSettings::default()
    };

    store.save(&settings).unwrap();

    let expected = [
        "  \"recurringAppointments\": [],",
        "  \"appointmentProposals\": {",
        "    \"enabled\": false,",
        "    \"dismissed\": [",
        "      {",
        "        \"weekday\": \"thursday\",",
        "        \"start\": \"14:00:00\",",
        "        \"end\": \"16:00:00\",",
        "        \"until\": \"2026-11-05\"",
        "      },",
        "      {",
        "        \"weekday\": \"monday\",",
        "        \"start\": \"09:00:00\",",
        "        \"end\": \"09:30:00\",",
        "        \"until\": null",
        "      }",
        "    ]",
        "  },",
    ]
    .join(NEWLINE);
    assert!(
        fixture.contents().contains(&expected),
        "{}",
        fixture.contents()
    );
    assert_eq!(store.load().unwrap().settings, settings);
}

#[test]
fn a_recurring_appointment_matches_in_any_case_and_an_unknown_weekday_is_rejected() {
    let fixture = Fixture::with_file(
        r#"{"RecurringAppointments":[{"ID":"a","Weekday":"Friday","Start":"9:30","End":"10:00"}]}"#,
    );
    let appointments = fixture.load().settings.recurring_appointments;
    assert_eq!(appointments.len(), 1);
    assert_eq!(appointments[0].weekday, Weekday::Friday);
    assert_eq!(appointments[0].start, Time::constant(9, 30, 0, 0));
    assert!(appointments[0].enabled);

    let fixture = Fixture::with_file(
        r#"{"recurringAppointments":[{"id":"a","weekday":"jeudi","start":"14:00","end":"16:00"}]}"#,
    );
    assert!(matches!(
        fixture.load().source,
        SettingsSource::Recovered { .. }
    ));
}
