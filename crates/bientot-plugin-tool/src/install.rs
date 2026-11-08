//! `install`: installs a package in a Bientôt data folder as the consent dialog's « Installer »
//! does, turned on with what it asks for. It is for measurements and tests: users install from
//! Settings › Plugins, which shows them what the plugin asks for first.

use std::path::Path;

use bientot_plugins::files::PluginsDir;
use bientot_plugins::manager;
use bientot_plugins::runtime::Runtime;
use bientot_plugins::state::StateFile;

use crate::{Args, read};

pub fn run(args: &Args) -> Result<(), String> {
    let path = args.path(0, "package")?;
    let data = args
        .option("--data-dir")
        .ok_or("missing --data-dir <Bientôt data folder>")?;
    let dir = PluginsDir::new(Path::new(data).join("plugins"));
    std::fs::create_dir_all(dir.root()).map_err(|e| format!("{}: {e}", dir.root().display()))?;
    let state = dir.state_path();
    let mut state = match state.exists() {
        true => StateFile::load(&state).map_err(|e| format!("{}: {e}", state.display()))?,
        false => StateFile::default(),
    };
    let offer = manager::offer(&read(&path)?, &[], &Runtime::default(), &state)
        .map_err(|e| e.to_string())?;
    let version = offer.package.manifest.version.to_string();
    let id = manager::install(&dir, &mut state, offer).map_err(|e| e.to_string())?;
    println!(
        "installed {} {version} in {}, turned on",
        id.as_str(),
        dir.root().display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use bientot_plugins::state::Status;

    use super::*;
    use crate::pack::tests::{MODULE_WAT, folder};

    #[test]
    fn a_package_installs_turned_on_where_bientot_reads_it() {
        let (dir, bytes, manifest) = folder(r#""read:day""#);
        let module = wat::parse_str(MODULE_WAT).unwrap();
        let package = dir.path().join("p.bientot-plugin");
        std::fs::write(
            &package,
            crate::pack::pack(dir.path(), &module, &bytes, &manifest).unwrap(),
        )
        .unwrap();
        let data = dir.path().join("data");
        let args = Args {
            positional: vec![package.display().to_string()],
            options: vec![("--data-dir".into(), Some(data.display().to_string()))],
        };
        run(&args).unwrap();
        let plugins = PluginsDir::new(data.join("plugins"));
        let state = StateFile::load(&plugins.state_path()).unwrap();
        assert_eq!(state.schema, bientot_plugins::state::SCHEMA);
        let installed = &state.plugins[&manifest.id];
        assert_eq!(installed.status, Status::Active);
        assert!(
            installed
                .granted
                .capabilities
                .contains(&bientot_plugin_api::Capability::ReadDay)
        );
        // Bientôt loads it as it is.
        let (specs, problems) = manager::load_active(&plugins, &mut state.clone(), &[]);
        assert_eq!((specs.len(), problems.len()), (1, 0));
        // A second install is an update of the same plugin.
        run(&args).unwrap();
        assert_eq!(
            StateFile::load(&plugins.state_path())
                .unwrap()
                .plugins
                .len(),
            1
        );
    }
}
