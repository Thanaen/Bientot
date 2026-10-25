//! Runs the generator on this workspace, as the packaging script does. Cargo reads the registry
//! cache and downloads only the Windows-only crates a Linux build has not fetched.

use std::fs;
use std::path::Path;
use std::process::Command;

/// The notices written for bientot-app's `feature`, or its `release` one.
fn notices(feature: Option<&str>) -> String {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "THIRD-PARTY-NOTICES-{}.txt",
        feature.unwrap_or("release")
    ));
    let _ = fs::remove_file(&output);
    let run = Command::new(env!("CARGO_BIN_EXE_bientot-notices"))
        .arg(&output)
        .args(feature)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    fs::read_to_string(&output).unwrap()
}

fn lists(notices: &str, name: &str) -> bool {
    notices
        .lines()
        .any(|line| line.starts_with(&format!("{name} ")) && line.contains(" — "))
}

#[test]
fn writes_the_notices_of_the_release_exe() {
    let notices = notices(None);
    let listed = |name| lists(&notices, name);

    assert!(listed("gpui-pre"));
    assert!(listed("rusqlite"));
    assert!(listed("libsqlite3-sys"));
    assert!(notices.contains("in the public domain (https://sqlite.org/copyright.html)"));
    assert!(listed("ureq"), "the release features are on");
    assert!(!listed("syn"), "proc-macro dependencies are not in the exe");
    assert!(
        !listed("bientot-app"),
        "workspace crates are not third-party"
    );
    assert!(
        !listed("velopack"),
        "the Store package and the ZIP do not update through Velopack"
    );
}

#[test]
fn the_velopack_release_lists_velopack() {
    let notices = notices(Some("direct"));

    assert!(lists(&notices, "velopack"));
    assert!(lists(&notices, "ureq"));
}
