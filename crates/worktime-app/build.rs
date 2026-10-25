#[path = "build/version_resource.rs"]
mod version_resource;

use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build/version_resource.rs");
    println!("cargo:rerun-if-env-changed=WORKTIME_VERSION");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // The same version as Paramètres shows (settings_view::build).
    let version = env::var("WORKTIME_VERSION")
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").expect("Cargo sets the version"));
    let source = version_resource::rc(&version).unwrap_or_else(|error| panic!("{error}"));
    let rc = Path::new(&env::var("OUT_DIR").expect("Cargo sets OUT_DIR")).join("worktime.rc");
    std::fs::write(&rc, source).expect("the version resource is written to OUT_DIR");
    embed_resource::compile_for(&rc, ["worktime"], embed_resource::NONE)
        .manifest_required()
        .expect("the version resource compiles");
}
