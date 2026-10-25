//! Names from the time the app was called WorkTime that installed copies, their registry entries
//! and their data still carry. They are identities, not the app's name, and must never follow a
//! rename: each one is looked for, spelled exactly, in the file that defines it.

const PINNED: &[(&str, &str, &str)] = &[
    (
        "the exe that the Start menu shortcut, the Run value and Velopack's mainExe name",
        include_str!("../Cargo.toml"),
        "name = \"worktime\"\npath = \"src/main.rs\"",
    ),
    (
        "the Velopack pack id and main exe installed copies update by",
        include_str!("../../../scripts/release-direct.sh"),
        "--packId Thanaen.WorkTime ",
    ),
    (
        "the exe Velopack starts after an update",
        include_str!("../../../scripts/release-direct.sh"),
        "--mainExe worktime.exe ",
    ),
    (
        "the update feed installed copies read",
        include_str!("../src/updates/direct.rs"),
        "pub const FEED_URL: &str = \"https://worktime.thanaen.dev/updates/\";",
    ),
    (
        "the usage signal endpoint, which follows no redirect",
        include_str!("../../bientot-core/src/usage_ping.rs"),
        "pub const ENDPOINT: &str = \"https://worktime.thanaen.dev/v1/ping\";",
    ),
    (
        "the DPAPI entropy the saved ActivityWatch key is encrypted with",
        include_str!("../../bientot-store/src/secrets.rs"),
        "format!(\"WorkTime:{}\", self.0)",
    ),
    (
        "the table inside every export",
        include_str!("../../bientot-store/src/transfer.rs"),
        "CREATE TABLE WorkTimeTransfer (",
    ),
    (
        "the extension of the exports and backups made before the rename",
        include_str!("../../bientot-store/src/transfer.rs"),
        "[EXTENSION, \"worktime\"]",
    ),
    (
        "the folder the data moves from",
        include_str!("../../bientot-store/src/data_migration.rs"),
        "local_app_data.join(\"WorkTime\")",
    ),
    (
        "the Run value that starts the app with Windows",
        include_str!("../../bientot-windows/src/startup.rs"),
        "pub const REGISTRATION_NAME: &str = \"WorkTime\";",
    ),
    (
        "the argument the Run values already written pass",
        include_str!("../../bientot-windows/src/startup.rs"),
        "pub const WINDOWS_STARTUP_ARGUMENT: &str = \"--worktime-windows-startup\";",
    ),
    (
        "the portable copy's AppUserModelID, which holds the user's notification settings",
        include_str!("../../bientot-windows/src/notifications/portable.rs"),
        "const PORTABLE_AUMID: &str = \"Thanaen.WorkTime.Portable\";",
    ),
    (
        "the portable copy's toast activator",
        include_str!("../../bientot-windows/src/notifications/portable.rs"),
        "const PORTABLE_ACTIVATOR: u128 = 0x8ddae643_7db0_465f_9139_725c4f57f1ed;",
    ),
    (
        "the MSIX toast activator",
        include_str!("../../bientot-windows/src/notifications.rs"),
        "pub const PACKAGED_ACTIVATOR: u128 = 0x6cb667aa_56d8_4f29_b21c_7d6f329b4463;",
    ),
    (
        "the log files' names",
        include_str!("../../bientot-core/src/diagnostics/logger.rs"),
        "\"worktime-{}-{:07}-{}.jsonl\"",
    ),
    (
        "the prefix log rotation matches, which would leave older logs behind",
        include_str!("../../bientot-core/src/diagnostics/logger.rs"),
        "lower.starts_with(\"worktime-\")",
    ),
    (
        "the prefix the diagnostic export matches",
        include_str!("../../bientot-core/src/diagnostics/export.rs"),
        ".strip_prefix(\"worktime-\")",
    ),
];

#[test]
fn the_identities_from_the_worktime_name_are_unchanged() {
    let missing: Vec<_> = PINNED
        .iter()
        .filter(|(_, source, literal)| !source.contains(literal))
        .map(|(what, _, literal)| format!("{what}: {literal}"))
        .collect();

    assert!(
        missing.is_empty(),
        "renamed or moved:\n{}",
        missing.join("\n")
    );
}
