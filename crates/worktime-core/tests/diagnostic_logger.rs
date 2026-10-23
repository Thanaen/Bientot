//! Ported from `LocalDiagnosticLoggerTests.cs`. Each test reads the files the logger wrote and
//! compares whole lines. `writes_the_lines_dotnet_wrote` replays lines that the .NET serializer
//! wrote into `tests/fixtures/diagnostics/`, and `redacts_generated_texts_as_dotnet_does` replays
//! `tests/fixtures/redaction.jsonl`. Both were generated from the .NET app before its removal.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use jiff::{SignedDuration, Timestamp};
use tempfile::TempDir;
use worktime_core::diagnostics::{
    DiagnosticLogError, DiagnosticLogLevel, DiagnosticLogOptions, DiagnosticValue,
    LocalDiagnosticLogger,
};
use worktime_domain::configuration::{LoggingSettings, WorkTimeSettings};
use worktime_domain::time::Clock;

struct TestClock(Cell<Timestamp>);

impl TestClock {
    fn at(now: &str) -> Self {
        Self(Cell::new(now.parse().unwrap()))
    }

    fn advance(&self, duration: SignedDuration) {
        self.0.set(self.0.get() + duration);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        self.0.get()
    }
}

fn clock() -> TestClock {
    TestClock::at("2026-08-07T07:00:00Z")
}

fn files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
}

fn single_log(directory: &Path) -> String {
    let files = files(directory);
    assert_eq!(files.len(), 1, "{files:?}");
    fs::read_to_string(&files[0]).unwrap()
}

fn write_one(
    level: DiagnosticLogLevel,
    event: &str,
    properties: &[(&str, DiagnosticValue)],
) -> String {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("logs");
    let clock = clock();
    let mut logger =
        LocalDiagnosticLogger::new(&directory, DiagnosticLogOptions::default(), &clock);
    logger.write(level, event, properties).unwrap();
    single_log(&directory)
}

#[test]
fn emits_structured_json_and_redacts_sensitive_data() {
    let log = write_one(
        DiagnosticLogLevel::Warning,
        "activitywatch.connection_failed",
        &[
            ("attempt", 2.into()),
            ("apiKey", "top-secret-api-key".into()),
            ("window_title", "Private document".into()),
            ("title", "ActivityWatch current window".into()),
            (
                "details",
                "Authorization: Bearer abc.def and token=hidden-value".into(),
            ),
        ],
    );

    assert_eq!(
        log,
        r#"{"TimestampUtc":"2026-08-07T07:00:00+00:00","Level":"Warning","Event":"activitywatch.connection_failed","Properties":{"attempt":2,"apiKey":"[REDACTED]","window_title":"[REDACTED]","title":"[REDACTED]","details":"Authorization: Bearer [REDACTED] and token[REDACTED]"}}"#
            .to_owned()
            + "\n"
    );
    for secret in [
        "top-secret-api-key",
        "Private document",
        "ActivityWatch current window",
        "abc.def",
        "hidden-value",
    ] {
        assert!(!log.contains(secret), "{secret}");
    }
}

#[test]
fn redacts_secret_property_aliases() {
    for name in [
        "api-key",
        "API_KEY",
        "Authorization",
        "access.token",
        "client_secret",
    ] {
        let log = write_one(
            DiagnosticLogLevel::Warning,
            "activitywatch.authentication_failed",
            &[
                (name, "fictional-private-token".into()),
                ("status", 401.into()),
            ],
        );

        assert_eq!(
            log,
            format!(
                r#"{{"TimestampUtc":"2026-08-07T07:00:00+00:00","Level":"Warning","Event":"activitywatch.authentication_failed","Properties":{{"{name}":"[REDACTED]","status":401}}}}"#
            ) + "\n"
        );
    }
}

#[test]
fn redacts_credentials_inside_diagnostic_text() {
    for (details, expected) in [
        (
            "Authorization: bEaReR fictional-private-token",
            "Authorization: Bearer [REDACTED]",
        ),
        (
            "api_key=fictional-private-token&attempt=2",
            "api_key[REDACTED]\\u0026attempt=2",
        ),
        (
            "token: fictional-private-token; retry",
            "token[REDACTED]; retry",
        ),
    ] {
        let log = write_one(
            DiagnosticLogLevel::Warning,
            "activitywatch.authentication_failed",
            &[("details", details.into())],
        );

        assert_eq!(
            log,
            format!(
                r#"{{"TimestampUtc":"2026-08-07T07:00:00+00:00","Level":"Warning","Event":"activitywatch.authentication_failed","Properties":{{"details":"{expected}"}}}}"#
            ) + "\n"
        );
        assert!(!log.contains("fictional-private-token"));
    }
}

#[test]
fn redacts_exception_messages() {
    let message = r"Could not process customer-key-7f3a from window-payroll-q4 at C:\Users\Casey\Documents\private-ledger.db in Ledger Desktop Private.";
    for name in ["message", "MeS-sage"] {
        let log = write_one(
            DiagnosticLogLevel::Error,
            "periodic_refresh.failed",
            &[
                ("exceptionType", "System.IO.IOException".into()),
                (name, message.into()),
                ("attempt", 3.into()),
                ("status", 503.into()),
            ],
        );

        assert_eq!(
            log,
            format!(
                r#"{{"TimestampUtc":"2026-08-07T07:00:00+00:00","Level":"Error","Event":"periodic_refresh.failed","Properties":{{"exceptionType":"System.IO.IOException","{name}":"[REDACTED]","attempt":3,"status":503}}}}"#
            ) + "\n"
        );
        for marker in [
            "customer-key-7f3a",
            "window-payroll-q4",
            "private-ledger.db",
            "Ledger Desktop Private",
        ] {
            assert!(!log.contains(marker), "{marker}");
        }
    }
}

#[test]
fn saved_log_limits_are_accepted_by_settings_exactly_when_the_logger_accepts_them() {
    for (file_count, file_bytes, expected) in [
        (1, 4_096, true),
        (5, 1_048_576, true),
        (50, 20 * 1024 * 1024, true),
        (0, 1_048_576, false),
        (5, 4_095, false),
        (5, 20 * 1024 * 1024 + 1, false),
    ] {
        let logging = LoggingSettings {
            maximum_file_count: file_count,
            maximum_file_bytes: file_bytes,
        };
        let settings = WorkTimeSettings {
            logging,
            ..WorkTimeSettings::default()
        };

        assert_eq!(
            settings.validate().is_empty(),
            expected,
            "{file_count} files of {file_bytes} bytes"
        );
        assert_eq!(
            DiagnosticLogOptions::from_settings(logging).is_some(),
            expected,
            "{file_count} files of {file_bytes} bytes"
        );
    }
}

#[test]
fn rotates_and_prunes_to_configured_bounds() {
    let root = TempDir::new().unwrap();
    let clock = clock();
    let options = DiagnosticLogOptions::new(2, 4_096, 8_192).unwrap();
    let mut logger = LocalDiagnosticLogger::new(root.path(), options, &clock);
    let padding = "x".repeat(1_900);

    for _ in 0..8 {
        logger
            .write(
                DiagnosticLogLevel::Information,
                "refresh.completed",
                &[("padding", padding.as_str().into())],
            )
            .unwrap();
    }

    let files = files(root.path());
    let sizes: Vec<u64> = files
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(sizes.iter().sum::<u64>() <= 8_192, "{sizes:?}");
    assert!(sizes.iter().all(|size| *size <= 4_096), "{sizes:?}");
}

#[test]
fn removes_files_older_than_fourteen_days() {
    let root = TempDir::new().unwrap();
    let clock = clock();
    let mut logger =
        LocalDiagnosticLogger::new(root.path(), DiagnosticLogOptions::default(), &clock);

    logger
        .write(DiagnosticLogLevel::Information, "app.started", &[])
        .unwrap();
    let original = files(root.path());
    assert_eq!(original.len(), 1);

    clock.advance(SignedDuration::from_hours(13 * 24));
    logger
        .write(DiagnosticLogLevel::Information, "app.heartbeat", &[])
        .unwrap();
    assert_eq!(files(root.path()), original);

    clock.advance(SignedDuration::from_hours(2 * 24));
    logger
        .write(DiagnosticLogLevel::Information, "app.restarted", &[])
        .unwrap();

    let retained = files(root.path());
    assert_eq!(retained.len(), 1);
    assert_ne!(retained, original);
    assert_eq!(
        fs::read_to_string(&retained[0]).unwrap(),
        "{\"TimestampUtc\":\"2026-08-22T07:00:00+00:00\",\"Level\":\"Information\",\"Event\":\"app.restarted\",\"Properties\":null}\n"
    );
}

#[test]
fn replaces_non_finite_floating_point_values() {
    let log = write_one(
        DiagnosticLogLevel::Information,
        "metrics.sampled",
        &[
            ("single", f64::NAN.into()),
            ("double", f64::INFINITY.into()),
        ],
    );

    assert_eq!(
        log,
        r#"{"TimestampUtc":"2026-08-07T07:00:00+00:00","Level":"Information","Event":"metrics.sampled","Properties":{"single":"[NON_FINITE]","double":"[NON_FINITE]"}}"#
            .to_owned()
            + "\n"
    );
}

#[test]
fn rejects_event_names_that_could_contain_user_data() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("logs");
    let clock = clock();
    let mut logger =
        LocalDiagnosticLogger::new(&directory, DiagnosticLogOptions::default(), &clock);

    for event in ["window title from user", "event/with/payload"] {
        assert!(matches!(
            logger.write(DiagnosticLogLevel::Information, event, &[]),
            Err(DiagnosticLogError::InvalidEventName)
        ));
    }
    assert!(!directory.exists());
}

#[test]
fn writes_the_lines_dotnet_wrote() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics");
    let update = fs::read_to_string(fixtures.join("update/input.jsonl")).unwrap();
    let hostile = fs::read_to_string(fixtures.join("hostile/input.jsonl")).unwrap();
    let expected: Vec<&str> = vec![
        update.lines().nth(1).unwrap(),
        update.lines().nth(2).unwrap(),
        update.lines().nth(3).unwrap(),
        hostile.lines().nth(1).unwrap(),
    ];
    let root = TempDir::new().unwrap();
    let clock = TestClock::at("2026-09-10T12:32:56Z");
    let mut logger =
        LocalDiagnosticLogger::new(root.path(), DiagnosticLogOptions::default(), &clock);

    logger
        .write(
            DiagnosticLogLevel::Warning,
            "update_check.unavailable",
            &[
                ("exceptionType", "System.InvalidOperationException".into()),
                ("hresult", 5.into()),
            ],
        )
        .unwrap();
    clock.advance(SignedDuration::from_mins(-1));
    logger
        .write(DiagnosticLogLevel::Information, "startup.tray_ready", &[])
        .unwrap();
    clock.advance(SignedDuration::from_mins(-1));
    logger
        .write(
            DiagnosticLogLevel::Error,
            "periodic_refresh.failed",
            &[
                ("exceptionType", "System.IO.IOException".into()),
                ("hresult", 7.into()),
            ],
        )
        .unwrap();
    clock.advance(SignedDuration::from_mins(2));
    logger
        .write(
            DiagnosticLogLevel::Warning,
            "power_history.recovery_unavailable",
            &[
                ("status", "Truncated".into()),
                ("reason", "SENTINEL-window-title".into()),
            ],
        )
        .unwrap();

    assert_eq!(
        single_log(root.path()).lines().collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn escapes_text_and_keeps_the_tick_in_name_and_timestamp() {
    let root = TempDir::new().unwrap();
    let clock = TestClock::at("2026-08-07T07:00:00.1234500Z");
    let mut logger =
        LocalDiagnosticLogger::new(root.path(), DiagnosticLogOptions::default(), &clock);

    logger
        .write(
            DiagnosticLogLevel::Debug,
            "app.started",
            &[
                ("details", "\"a\\b\"\n\t<&'+>`/é😀\u{1}\u{7f}".into()),
                ("count", u64::MAX.into()),
                ("ratio", 0.25.into()),
                ("ready", true.into()),
                ("reason", DiagnosticValue::Null),
            ],
        )
        .unwrap();

    let files = files(root.path());
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with("worktime-20260807-070000-1234500-") && name.ends_with(".jsonl"),
        "{name}"
    );
    assert_eq!(
        fs::read_to_string(&files[0]).unwrap(),
        r#"{"TimestampUtc":"2026-08-07T07:00:00.12345+00:00","Level":"Debug","Event":"app.started","Properties":{"details":"\u0022a\\b\u0022\n\t\u003C\u0026\u0027\u002B\u003E\u0060/\u00E9\uD83D\uDE00\u0001\u007F","count":18446744073709551615,"ratio":0.25,"ready":true,"reason":null}}"#
            .to_owned()
            + "\n"
    );
}

#[test]
fn replaces_an_entry_larger_than_a_file() {
    let root = TempDir::new().unwrap();
    let clock = clock();
    let options = DiagnosticLogOptions::new(1, 4_096, 4_096).unwrap();
    let mut logger = LocalDiagnosticLogger::new(root.path(), options, &clock);
    let padding = "x".repeat(2_048);

    logger
        .write(
            DiagnosticLogLevel::Warning,
            "refresh.completed",
            &[
                ("first", padding.as_str().into()),
                ("second", padding.as_str().into()),
            ],
        )
        .unwrap();

    assert_eq!(
        single_log(root.path()),
        r#"{"TimestampUtc":"2026-08-07T07:00:00+00:00","Level":"Warning","Event":"refresh.completed","Properties":{"loggingFailure":"entry_too_large"}}"#
            .to_owned()
            + "\n"
    );
}

#[test]
fn redacts_generated_texts_as_dotnet_does() {
    let pieces = [
        "Bearer",
        "bearer",
        "BEARER",
        "bEaReR",
        " ",
        "  ",
        "\t",
        "\u{3000}",
        "\u{a0}",
        "\u{85}",
        "\u{e9}",
        "\u{200d}",
        "\u{200c}",
        "\u{300}",
        "\u{903}",
        "\u{663}",
        "\u{b2}",
        "_",
        "x",
        "Z",
        "9",
        "token",
        "to\u{212a}en",
        "TOKEN",
        "api_key",
        "api-key",
        "apikey",
        "API",
        "key",
        "password",
        "secret",
        "\u{17f}ecret",
        "ap\u{130}key",
        "=",
        ":",
        "&",
        ";",
        ",",
        "abc.def~+/=-",
        "/",
        "\u{212a}",
        "\u{17f}",
        "\u{131}",
        "\u{130}",
        "\u{1f600}",
        "\u{1d400}",
        "Bearer ",
        "Bearer tok",
        "token=",
        "password : ",
        "api-key:",
        "secret=v",
        "Token: t",
    ];
    let mut state: u32 = 0x2545_F491;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let expected = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/redaction.jsonl"),
    )
    .unwrap();
    let mut expected = expected.lines();
    for _ in 0..3_000 {
        let count = 1 + next() % 8;
        let text: String = (0..count)
            .map(|_| pieces[(next() % pieces.len() as u32) as usize])
            .collect();
        let root = TempDir::new().unwrap();
        let clock = TestClock::at("2026-09-10T12:34:56Z");
        let mut logger =
            LocalDiagnosticLogger::new(root.path(), DiagnosticLogOptions::default(), &clock);
        logger
            .write(
                DiagnosticLogLevel::Information,
                "probe",
                &[("details", text.as_str().into())],
            )
            .unwrap();
        assert_eq!(
            single_log(root.path()).trim_end(),
            expected.next().unwrap(),
            "{text:?}"
        );
    }
    assert_eq!(expected.next(), None);
}
