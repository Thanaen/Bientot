//! `pack`: a plugin folder and its module into a `.bientot-plugin`, with every check Bientôt makes
//! at install that does not need the app (the built-in command titles are checked at install).

use std::path::Path;

use bientot_plugin_api::{Language, LocaleTexts, Manifest};
use bientot_plugins::package::{MANIFEST, MODULE, Package};
use bientot_plugins::runtime::{ModuleError, Runtime};
use bientot_plugins::zip;

use crate::{Args, read, write};

pub fn run(args: &Args) -> Result<(), String> {
    let dir = args.path(0, "plugin folder")?;
    let wasm = args.path(1, "module (.wasm)")?;
    let manifest_bytes = read(&dir.join(MANIFEST))?;
    let manifest = Manifest::parse(&manifest_bytes, &[])
        .map_err(|e| format!("{MANIFEST}: {}", e.message(Language::En)))?;
    let out = match args.option("--out") {
        Some(out) => out.into(),
        None => dir.join(format!("{}.bientot-plugin", manifest.id.as_str())),
    };
    let bytes = pack(&dir, &read(&wasm)?, &manifest_bytes, &manifest)?;
    write(&out, &bytes)?;
    println!(
        "{} {} → {} ({} KiB, unsigned)",
        manifest.id.as_str(),
        manifest.version,
        out.display(),
        bytes.len().div_ceil(1024)
    );
    Ok(())
}

pub fn pack(
    dir: &Path,
    module: &[u8],
    manifest_bytes: &[u8],
    manifest: &Manifest,
) -> Result<Vec<u8>, String> {
    Runtime::default()
        .compile(module, &manifest.capabilities)
        .map_err(|e| format!("{e}{}", hint(&e)))?;
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (MANIFEST.to_owned(), manifest_bytes.to_vec()),
        (MODULE.to_owned(), module.to_vec()),
    ];
    for tag in &manifest.locales {
        let name = format!("locales/{tag}.json");
        let bytes = read(&dir.join(&name))?;
        LocaleTexts::parse(&bytes).map_err(|e| format!("{name}: {e:?}"))?;
        entries.push((name, bytes));
    }
    let assets = dir.join("assets");
    if assets.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(&assets)
            .map_err(|e| format!("{}: {e}", assets.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            entries.push((format!("assets/{name}"), read(&assets.join(&name))?));
        }
    }
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let bytes = zip::write(&refs);
    // The package reader is the one Bientôt uses: what it refuses here, Bientôt refuses too.
    Package::read(&bytes, &[]).map_err(|e| e.message(Language::En))?;
    Ok(bytes)
}

/// What to do about a refused module.
fn hint(error: &ModuleError) -> &'static str {
    match error {
        ModuleError::NotGranted(_) => "; add it to `capabilities` in plugin.json",
        ModuleError::MissingExport(_) | ModuleError::WrongExport(_) => {
            "; call `bientot_plugin_sdk::export!` once and build the crate as a `cdylib`"
        }
        ModuleError::UnknownImport(name) if name.starts_with("wasi") => {
            "; build for wasm32-unknown-unknown, not wasm32-wasip1"
        }
        ModuleError::UnknownImport(_) => {
            "; plugins import only bt:log@1, bt:time@1, bt:day@1 and bt:ui@1"
        }
        ModuleError::Invalid(_) | ModuleError::BadAbi(_) => "",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MODULE_WAT: &str = r#"(module
        (import "bt:day@1" "current" (func (param i32 i32) (result i64)))
        (memory (export "memory") 1)
        (func (export "bt_abi") (result i32) i32.const 1)
        (func (export "bt_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "bt_free") (param i32 i32))
        (func (export "bt_handle") (param i32 i32) (result i64) i64.const 0))"#;

    pub(crate) fn folder(capabilities: &str) -> (tempfile::TempDir, Vec<u8>, Manifest) {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!(
            r#"{{ "id": "test.sample", "name": {{ "en": "Sample" }}, "version": "1.2.3",
                 "api": "1.0", "locales": ["en"], "capabilities": [{capabilities}] }}"#
        );
        std::fs::write(dir.path().join(MANIFEST), &manifest).unwrap();
        std::fs::create_dir(dir.path().join("locales")).unwrap();
        std::fs::write(dir.path().join("locales/en.json"), r#"{ "hi": "Hi" }"#).unwrap();
        let parsed = Manifest::parse(manifest.as_bytes(), &[]).unwrap();
        (dir, manifest.into_bytes(), parsed)
    }

    #[test]
    fn a_module_that_needs_an_undeclared_capability_is_refused_with_the_fix() {
        let (dir, bytes, manifest) = folder("");
        let error = pack(
            dir.path(),
            &wat::parse_str(MODULE_WAT).unwrap(),
            &bytes,
            &manifest,
        )
        .unwrap_err();
        assert!(error.contains("`read:day`"), "{error}");
        assert!(error.contains("add it to `capabilities`"), "{error}");
    }

    #[test]
    fn a_complete_folder_packs_into_a_package_bientot_reads() {
        let (dir, bytes, manifest) = folder(r#""read:day""#);
        let module = wat::parse_str(MODULE_WAT).unwrap();
        let package = pack(dir.path(), &module, &bytes, &manifest).unwrap();
        let read = Package::read(&package, &[]).unwrap();
        assert_eq!(read.module(), module.as_slice());
        assert!(read.locales.contains_key("en"));
        assert!(read.signature.is_none());
        // The same files give the same package.
        assert_eq!(
            pack(dir.path(), &module, &bytes, &manifest).unwrap(),
            package
        );
    }
}
