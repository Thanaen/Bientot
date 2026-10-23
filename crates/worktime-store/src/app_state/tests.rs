use jiff::civil::date;
use tempfile::TempDir;

use super::*;

fn store(directory: &TempDir) -> AppStateStore {
    AppStateStore::new(directory.path().join("app-state.json")).unwrap()
}

#[test]
fn a_missing_file_reads_as_never_sent() {
    let directory = TempDir::new().unwrap();

    assert_eq!(store(&directory).load().unwrap(), AppState::default());
}

#[test]
fn the_last_send_date_is_written_alone_and_read_back() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    let state = AppState {
        usage_ping_last_sent: Some(date(2026, 10, 13)),
    };

    store.save(&state).unwrap();

    assert_eq!(
        fs::read_to_string(store.path()).unwrap(),
        "{\n  \"usagePingLastSent\": \"2026-10-13\"\n}\n"
    );
    assert_eq!(store.load().unwrap(), state);
}

#[test]
fn a_save_creates_the_directory_and_leaves_only_the_state_file() {
    let directory = TempDir::new().unwrap();
    let store =
        AppStateStore::new(directory.path().join("WorkTime").join("app-state.json")).unwrap();

    store
        .save(&AppState {
            usage_ping_last_sent: Some(date(2026, 1, 2)),
        })
        .unwrap();

    let names: Vec<_> = fs::read_dir(directory.path().join("WorkTime"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["app-state.json"]);
}

#[test]
fn an_unreadable_file_reads_as_never_sent() {
    for contents in [
        "",
        "not json",
        "[]",
        "{}",
        r#"{"usagePingLastSent": 20261013}"#,
        r#"{"usagePingLastSent": "2026-13-01"}"#,
        r#"{"usagePingLastSent": "2026-10-13T08:00"}"#,
        r#"{"usagePingLastSent": "13/10/2026"}"#,
    ] {
        let directory = TempDir::new().unwrap();
        let store = store(&directory);
        fs::write(store.path(), contents).unwrap();

        assert_eq!(store.load().unwrap(), AppState::default(), "{contents:?}");
    }
}

#[test]
fn a_null_date_reads_as_never_sent() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    fs::write(store.path(), r#"{"usagePingLastSent": null}"#).unwrap();

    assert_eq!(store.load().unwrap(), AppState::default());
}
