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
        ..AppState::default()
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
            ..AppState::default()
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

#[test]
fn the_callouts_closed_today_are_written_beside_the_last_send_and_read_back() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    let state = AppState {
        usage_ping_last_sent: Some(date(2026, 10, 13)),
        closed_callouts: ClosedCallouts {
            date: Some(date(2026, 10, 24)),
            counts: vec!["automatic:thursday".into()],
            offers: vec!["thursday-1000".into()],
        },
    };

    store.save(&state).unwrap();

    let written: Value = serde_json::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(
        written,
        serde_json::json!({
            "usagePingLastSent": "2026-10-13",
            "closedCallouts": {
                "date": "2026-10-24",
                "counts": ["automatic:thursday"],
                "offers": ["thursday-1000"],
            },
        })
    );
    assert_eq!(store.load().unwrap(), state);
    let closed = &state.closed_callouts;
    assert_eq!(
        closed.on(date(2026, 10, 24)),
        (&closed.counts[..], &closed.offers[..])
    );
    assert_eq!(closed.on(date(2026, 10, 25)), (&[][..], &[][..]));
}

#[test]
fn an_update_keeps_what_another_writer_saved() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    store
        .update(|state| state.usage_ping_last_sent = Some(date(2026, 10, 13)))
        .unwrap();

    store
        .update(|state| {
            state.closed_callouts = ClosedCallouts {
                date: Some(date(2026, 10, 24)),
                counts: Vec::new(),
                offers: vec!["thursday-1000".into()],
            }
        })
        .unwrap();

    let state = store.load().unwrap();
    assert_eq!(state.usage_ping_last_sent, Some(date(2026, 10, 13)));
    assert_eq!(state.closed_callouts.offers, ["thursday-1000"]);
}

#[test]
fn unreadable_closed_callouts_read_as_none_closed_and_keep_the_last_send() {
    for closed in [
        "null",
        "[]",
        r#"{"counts": [], "offers": []}"#,
        r#"{"date": "2026-10-24", "counts": [1], "offers": []}"#,
        r#"{"date": "24/10/2026", "counts": [], "offers": []}"#,
    ] {
        let directory = TempDir::new().unwrap();
        let store = store(&directory);
        fs::write(
            store.path(),
            format!(r#"{{"usagePingLastSent": "2026-10-13", "closedCallouts": {closed}}}"#),
        )
        .unwrap();

        let state = store.load().unwrap();

        assert_eq!(
            state.usage_ping_last_sent,
            Some(date(2026, 10, 13)),
            "{closed}"
        );
        assert_eq!(state.closed_callouts, ClosedCallouts::default(), "{closed}");
    }
}
