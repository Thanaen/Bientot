//! Runs the generator on this workspace, as the packaging script does. Cargo reads the registry
//! cache and downloads only the Windows-only crates a Linux build has not fetched.

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn writes_the_notices_of_the_release_exe() {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join("THIRD-PARTY-NOTICES.txt");
    let _ = fs::remove_file(&output);
    let run = Command::new(env!("CARGO_BIN_EXE_worktime-notices"))
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let notices = fs::read_to_string(&output).unwrap();
    let listed = |name: &str| {
        notices
            .lines()
            .any(|line| line.starts_with(&format!("{name} ")) && line.contains(" — "))
    };
    assert!(listed("gpui-pre"));
    assert!(listed("rusqlite"));
    assert!(listed("libsqlite3-sys"));
    assert!(notices.contains("in the public domain (https://sqlite.org/copyright.html)"));
    assert!(listed("ureq"), "the release features are on");
    assert!(!listed("syn"), "proc-macro dependencies are not in the exe");
    assert!(
        !listed("worktime-app"),
        "workspace crates are not third-party"
    );
}
