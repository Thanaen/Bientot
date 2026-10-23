use jiff::SignedDuration;
use jiff::civil::Time;

use super::*;
use crate::configuration::ActivityWatchSettings;
use crate::sources::ActivitySourceKind;

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn seconds(seconds: i64) -> SignedDuration {
    SignedDuration::from_secs(seconds)
}

fn errors(settings: WorkTimeSettings) -> Vec<SettingsError> {
    settings.validate()
}

#[test]
fn defaults_match_ratified_product_decisions() {
    let settings = WorkTimeSettings::default();

    assert_eq!(settings.nominal_daily_target, SignedDuration::from_hours(7));
    assert_eq!(
        settings.minimum_effective_target,
        SignedDuration::from_hours(6)
    );
    assert_eq!(settings.lunch_start, Time::constant(12, 0, 0, 0));
    assert_eq!(settings.lunch_end, Time::constant(14, 0, 0, 0));
    assert_eq!(
        settings.preferred_latest_departure,
        Time::constant(18, 30, 0, 0)
    );
    assert_eq!(settings.orange_warning_margin, minutes(15));
    assert_eq!(settings.long_lock_review_threshold, minutes(90));
    assert!(settings.notifications.completion_approach.enabled);
    assert!(settings.notifications.latest_departure_approach.enabled);
    assert!(!settings.notifications.day_complete.enabled);
    assert!(settings.start_with_windows);
    assert_eq!(settings.activity_watch.base_url, "http://127.0.0.1:5600");
}

#[test]
fn defaults_match_ratified_schedule() {
    let settings = WorkTimeSettings::default();

    assert_eq!(settings.validate(), []);
    assert_eq!(settings.nominal_daily_target, SignedDuration::from_hours(7));
    assert_eq!(
        settings.minimum_effective_target,
        SignedDuration::from_hours(6)
    );
    assert_eq!(settings.lunch_start, Time::constant(12, 0, 0, 0));
    assert_eq!(settings.lunch_end, Time::constant(14, 0, 0, 0));
    assert_eq!(
        settings.preferred_latest_departure,
        Time::constant(18, 30, 0, 0)
    );
    assert_eq!(settings.orange_warning_margin, minutes(15));
    assert_eq!(settings.long_lock_review_threshold, minutes(90));
    let settlement = settings.activity_watch.settlement;
    assert_eq!(settlement.afk_timeout, seconds(180));
    assert_eq!(settlement.poll_interval, seconds(5));
    assert_eq!(settlement.maximum_jitter, seconds(5));
    assert_eq!(settlement.horizon(), seconds(190));
}

#[test]
fn rejects_invalid_target_and_floor() {
    for (target, minimum, error) in [
        (0, 360, SettingsError::Target),
        (1441, 360, SettingsError::Target),
        (360, 361, SettingsError::Minimum),
        (420, 0, SettingsError::Minimum),
    ] {
        let found = errors(WorkTimeSettings {
            nominal_daily_target: minutes(target),
            minimum_effective_target: minutes(minimum),
            ..WorkTimeSettings::default()
        });
        assert!(found.contains(&error), "{target}/{minimum}: {found:?}");
    }
}

#[test]
fn rejects_reversed_lunch_and_departure_during_lunch() {
    assert!(
        errors(WorkTimeSettings {
            lunch_end: Time::constant(11, 0, 0, 0),
            ..WorkTimeSettings::default()
        })
        .contains(&SettingsError::Lunch)
    );
    assert!(
        errors(WorkTimeSettings {
            preferred_latest_departure: Time::constant(14, 0, 0, 0),
            ..WorkTimeSettings::default()
        })
        .contains(&SettingsError::Departure)
    );
}

/// The .NET test also rejects null `ActivityWatch` and `Notifications`; Rust settings cannot
/// hold a missing group, so only the threshold cases remain.
#[test]
fn rejects_invalid_thresholds() {
    assert!(
        errors(WorkTimeSettings {
            long_lock_review_threshold: SignedDuration::ZERO,
            ..WorkTimeSettings::default()
        })
        .contains(&SettingsError::Lock)
    );
    assert!(
        errors(WorkTimeSettings {
            orange_warning_margin: minutes(-1),
            ..WorkTimeSettings::default()
        })
        .contains(&SettingsError::Margin)
    );
}

#[test]
fn rejects_a_debt_limit_outside_one_minute_to_one_day() {
    for limit in [0, -1, 1441] {
        let found = errors(WorkTimeSettings {
            weekly_debt_enabled: true,
            maximum_weekly_debt: Some(minutes(limit)),
            ..WorkTimeSettings::default()
        });
        assert!(found.contains(&SettingsError::Debt), "{limit}: {found:?}");
    }
}

#[test]
fn debt_is_off_without_limit_by_default_and_accepts_an_optional_limit() {
    assert!(!WorkTimeSettings::default().weekly_debt_enabled);
    assert_eq!(WorkTimeSettings::default().maximum_weekly_debt, None);
    assert_eq!(
        errors(WorkTimeSettings {
            weekly_debt_enabled: true,
            maximum_weekly_debt: Some(SignedDuration::from_hours(2)),
            ..WorkTimeSettings::default()
        }),
        []
    );
}

#[test]
fn allows_equal_floor_and_zero_warning_margin() {
    assert_eq!(
        errors(WorkTimeSettings {
            minimum_effective_target: SignedDuration::from_hours(7),
            orange_warning_margin: SignedDuration::ZERO,
            ..WorkTimeSettings::default()
        }),
        []
    );
}

#[test]
fn rejects_addresses_that_the_activity_watch_adapter_cannot_use() {
    for address in [
        "http://localhost:5600/api",
        "http://localhost:5600/?",
        "http://localhost:5600/#",
        "http://user:password@localhost:5600",
    ] {
        let found = errors(WorkTimeSettings {
            activity_watch: ActivityWatchSettings {
                base_url: address.to_owned(),
                ..ActivityWatchSettings::default()
            },
            ..WorkTimeSettings::default()
        });
        assert!(
            found.contains(&SettingsError::Source),
            "{address}: {found:?}"
        );
    }
}

#[test]
fn rejects_unsupported_settlement_timings() {
    for (timeout, poll, jitter) in [(0, 5, 5), (180, 0, 5), (180, 5, -1), (1800, 5, 5)] {
        let mut settings = WorkTimeSettings::default();
        settings.activity_watch.settlement = ActivityWatchSettlementContract {
            afk_timeout: seconds(timeout),
            poll_interval: seconds(poll),
            maximum_jitter: seconds(jitter),
        };
        let found = errors(settings);
        assert!(
            found.contains(&SettingsError::Source),
            "{timeout}/{poll}/{jitter}: {found:?}"
        );
    }
}

#[test]
fn defaults_leave_the_activity_source_unrecorded_with_a_three_minute_idle_threshold() {
    assert_eq!(WorkTimeSettings::default().activity_source, None);
    assert_eq!(
        WorkTimeSettings::default().built_in_activity.idle_threshold,
        seconds(180)
    );
}

#[test]
fn accepts_either_activity_source() {
    for source in [
        ActivitySourceKind::BuiltIn,
        ActivitySourceKind::ActivityWatch,
    ] {
        assert_eq!(
            errors(WorkTimeSettings {
                activity_source: Some(source),
                ..WorkTimeSettings::default()
            }),
            []
        );
    }
}

#[test]
fn accepts_an_idle_threshold_from_thirty_seconds_to_thirty_minutes() {
    for threshold in [30, 180, 1800] {
        assert_eq!(
            errors(WorkTimeSettings {
                built_in_activity: BuiltInActivitySettings {
                    idle_threshold: seconds(threshold),
                },
                ..WorkTimeSettings::default()
            }),
            [],
            "{threshold}"
        );
    }
}

#[test]
fn rejects_an_idle_threshold_outside_thirty_seconds_to_thirty_minutes() {
    for threshold in [0, 29, 1801] {
        assert_eq!(
            errors(WorkTimeSettings {
                built_in_activity: BuiltInActivitySettings {
                    idle_threshold: seconds(threshold),
                },
                ..WorkTimeSettings::default()
            }),
            [SettingsError::IdleThreshold],
            "{threshold}"
        );
    }
}

#[test]
fn errors_keep_the_dotnet_resource_keys() {
    let found = errors(WorkTimeSettings {
        schema_version: 2,
        language_override: Some("de-DE".to_owned()),
        logging: LoggingSettings {
            maximum_file_count: 0,
            maximum_file_bytes: 4_096,
        },
        ..WorkTimeSettings::default()
    });

    assert_eq!(
        found
            .iter()
            .map(|error| error.resource_key())
            .collect::<Vec<_>>(),
        ["SettingsSchema", "SettingsLanguage", "SettingsStorage"]
    );
}

#[test]
fn base_addresses_follow_the_dotnet_uri_rules() {
    let accepted = [
        "http://127.0.0.1:5600",
        "HTTP://LOCALHOST:5600",
        " http://h ",
        "http://h/./",
        "http://h/a/..",
        "http://h:5600/%2e",
        "http://@h",
        "http://h:",
        "http://[::1]:5600",
        "http://[::ffff:127.0.0.1]:5600",
        "http://[fe80::1%25eth0]:5600",
        "http://[::FFFF:7F00:1]/",
        "http://a.b.",
        "http://é.fr",
    ];
    let rejected = [
        "http://h//",
        "http://h:65536",
        "http://h:+80",
        "ftp://h",
        "http:/h",
        "http://",
        "http://h h",
        "http://[::1",
        "http://[:::]:5600",
        "http://[1:2:3]:5600",
        "http://[12345::]:5600",
        "http://[1::2::3]:5600",
        "http://[::1.2.3]:5600",
        "http://[1:2:3:4:5:6:7:8:9]:5600",
        "http://h%41",
        "http://h..",
        "http://h~",
        "http://:5600",
        "http://h:5600:1",
    ];
    for address in accepted {
        assert!(is_usable_base_address(address), "{address}");
    }
    for address in rejected {
        assert!(!is_usable_base_address(address), "{address}");
    }
}

#[test]
fn recurring_appointments_need_a_unique_plain_id_a_weekday_and_hours_in_order() {
    let thursday = RecurringAppointment {
        id: "thursday-meeting".to_owned(),
        name: "Réunion".to_owned(),
        weekday: jiff::civil::Weekday::Thursday,
        start: Time::constant(14, 0, 0, 0),
        end: Time::constant(16, 0, 0, 0),
        enabled: true,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    };
    let with = |appointments: Vec<RecurringAppointment>| {
        errors(WorkTimeSettings {
            recurring_appointments: appointments,
            ..WorkTimeSettings::default()
        })
    };
    let changed = |change: fn(&mut RecurringAppointment)| {
        let mut appointment = thursday.clone();
        change(&mut appointment);
        with(vec![appointment])
    };

    assert!(with(vec![thursday.clone()]).is_empty());
    assert!(
        changed(|appointment| {
            appointment.alternate_weeks_from = Some(jiff::civil::date(2026, 10, 29))
        })
        .is_empty()
    );
    for invalid in [
        changed(|appointment| appointment.id = String::new()),
        changed(|appointment| appointment.id = "a:b".to_owned()),
        changed(|appointment| appointment.weekday = jiff::civil::Weekday::Saturday),
        changed(|appointment| appointment.end = appointment.start),
        changed(|appointment| {
            appointment.alternate_weeks_from = Some(jiff::civil::date(2026, 10, 28))
        }),
        changed(|appointment| appointment.name = "a\nb".to_owned()),
        changed(|appointment| appointment.name = "x".repeat(81)),
        with(vec![thursday.clone(), thursday.clone()]),
        with(vec![thursday.clone(); 21]),
    ] {
        assert_eq!(invalid, [SettingsError::Appointments]);
    }
}
