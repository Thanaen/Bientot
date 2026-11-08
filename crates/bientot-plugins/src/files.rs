//! The `plugins\` folder: one folder per id, written whole through a temporary folder and a
//! rename, and checked against its recorded hashes at every load.

use std::collections::BTreeMap;
use std::fs;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bientot_plugin_api::PluginId;

use crate::package::{self, Package};

pub const STATE_FILE: &str = "state.json";

/// Replaces `path` atomically through `<path>.<random>.tmp`.
pub fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.tmp", random_hex()));
    let temporary = PathBuf::from(name);
    let written = (|| {
        let mut file = fs::File::create_new(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn random_hex() -> String {
    let state = RandomState::new();
    format!("{:016x}{:016x}", state.hash_one(1u8), state.hash_one(2u8))
}

#[derive(Debug, Clone)]
pub struct PluginsDir {
    root: PathBuf,
}

impl PluginsDir {
    pub fn new(root: PathBuf) -> PluginsDir {
        PluginsDir { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_path(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    pub fn plugin(&self, id: &PluginId) -> PathBuf {
        self.root.join(id.as_str())
    }

    /// Writes the package's files to a temporary folder, then swaps it in for `plugins\<id>\`.
    pub fn install(&self, package: &Package) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let id = &package.manifest.id;
        let temporary = self
            .root
            .join(format!(".new-{}-{}", id.as_str(), random_hex()));
        let written = (|| {
            for (path, data) in &package.files {
                let target = temporary.join(path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file = fs::File::create_new(&target)?;
                file.write_all(data)?;
                file.sync_all()?;
            }
            Ok(())
        })();
        if let Err(error) = written {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
        let target = self.plugin(id);
        // Windows does not rename a folder over another one: move the old one aside first.
        let old = self
            .root
            .join(format!(".old-{}-{}", id.as_str(), random_hex()));
        let had_old = match fs::rename(&target, &old) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => {
                let _ = fs::remove_dir_all(&temporary);
                return Err(e);
            }
        };
        if let Err(error) = fs::rename(&temporary, &target) {
            if had_old {
                let _ = fs::rename(&old, &target);
            }
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
        if had_old {
            let _ = fs::remove_dir_all(&old);
        }
        Ok(())
    }

    pub fn remove(&self, id: &PluginId) -> io::Result<()> {
        match fs::remove_dir_all(self.plugin(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Removes the leftovers of an install that stopped half way. An install that stopped between
    /// its two renames left the plugin only in `.old-<id>-…`: that folder goes back in place.
    pub fn sweep(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let orphan = name
                .strip_prefix(".old-")
                .and_then(|rest| rest.rsplit_once('-'))
                .and_then(|(id, _)| PluginId::parse(id))
                .filter(|id| !self.plugin(id).exists());
            if let Some(id) = orphan
                && fs::rename(entry.path(), self.plugin(&id)).is_ok()
            {
                continue;
            }
            if name.starts_with(".new-") || name.starts_with(".old-") {
                let _ = fs::remove_dir_all(entry.path());
            } else if name.ends_with(".tmp") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// Reads the plugin's files and checks them against `expected` (path → SHA-256 hex). Returns
    /// them when every file is there with its hash and nothing else is.
    pub fn load(
        &self,
        id: &PluginId,
        expected: &BTreeMap<String, String>,
    ) -> io::Result<Option<BTreeMap<String, Vec<u8>>>> {
        let folder = self.plugin(id);
        let mut found = BTreeMap::new();
        if !collect(&folder, "", &mut found)? || found.len() != expected.len() {
            return Ok(None);
        }
        for (path, data) in &found {
            if expected.get(path) != Some(&package::hex(&package::hash(data))) {
                return Ok(None);
            }
        }
        Ok(Some(found))
    }
}

/// Reads the regular files of `folder` and of its direct subfolders. Returns `false` when it holds
/// anything else, which the host never writes: a link, or a deeper folder.
fn collect(folder: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) -> io::Result<bool> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = format!("{prefix}{}", entry.file_name().to_string_lossy());
        let kind = entry.file_type()?;
        let plain = if kind.is_dir() && prefix.is_empty() {
            collect(&entry.path(), &format!("{path}/"), out)?
        } else if kind.is_file() {
            out.insert(path, fs::read(entry.path())?);
            true
        } else {
            false
        };
        if !plain {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::{build, entries, manifest_json};

    fn package(module: &[u8]) -> Package {
        Package::read(&build(&entries(&manifest_json(), module), None, &[]), &[]).unwrap()
    }

    fn expected(package: &Package) -> BTreeMap<String, String> {
        package
            .hashes()
            .iter()
            .map(|(p, h)| (p.clone(), package::hex(h)))
            .collect()
    }

    #[test]
    fn an_install_stopped_between_its_renames_is_undone_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = PluginsDir::new(dir.path().join("plugins"));
        let first = package(b"one");
        let id = first.manifest.id.clone();
        plugins.install(&first).unwrap();
        // The old folder was moved aside, the new one never came in.
        let aside = dir
            .path()
            .join(format!("plugins/.old-{}-0123abcd", id.as_str()));
        fs::rename(plugins.plugin(&id), &aside).unwrap();
        fs::create_dir(
            dir.path()
                .join(format!("plugins/.new-{}-4567", id.as_str())),
        )
        .unwrap();
        plugins.sweep();
        assert!(plugins.load(&id, &expected(&first)).unwrap().is_some());
        assert_eq!(fs::read_dir(dir.path().join("plugins")).unwrap().count(), 1);
        // With the plugin in place, a leftover old folder goes.
        fs::create_dir(&aside).unwrap();
        plugins.sweep();
        assert!(!aside.exists());
        assert!(plugins.load(&id, &expected(&first)).unwrap().is_some());
    }

    #[test]
    fn install_replace_load_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = PluginsDir::new(dir.path().join("plugins"));
        let first = package(b"one");
        let id = first.manifest.id.clone();
        plugins.install(&first).unwrap();
        assert_eq!(
            plugins.load(&id, &expected(&first)).unwrap().unwrap()["plugin.wasm"],
            b"one"
        );
        let second = package(b"two");
        plugins.install(&second).unwrap();
        assert!(plugins.load(&id, &expected(&first)).unwrap().is_none());
        assert!(plugins.load(&id, &expected(&second)).unwrap().is_some());
        let names: Vec<_> = fs::read_dir(plugins.root())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("bientot.focus")]);
        plugins.remove(&id).unwrap();
        plugins.remove(&id).unwrap();
        assert!(!plugins.plugin(&id).exists());
    }

    #[test]
    fn a_changed_or_added_file_fails_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = PluginsDir::new(dir.path().to_path_buf());
        let package = package(b"one");
        let id = package.manifest.id.clone();
        plugins.install(&package).unwrap();
        let folder = plugins.plugin(&id);
        fs::write(folder.join("plugin.wasm"), b"evil").unwrap();
        assert!(plugins.load(&id, &expected(&package)).unwrap().is_none());
        plugins.install(&package).unwrap();
        fs::write(folder.join("locales/xx.json"), b"{}").unwrap();
        assert!(plugins.load(&id, &expected(&package)).unwrap().is_none());
        plugins.install(&package).unwrap();
        fs::create_dir_all(folder.join("locales/deep")).unwrap();
        assert!(plugins.load(&id, &expected(&package)).unwrap().is_none());
    }

    #[test]
    fn leftovers_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = PluginsDir::new(dir.path().to_path_buf());
        fs::create_dir_all(dir.path().join(".new-x-1/locales")).unwrap();
        fs::write(dir.path().join("state.json.ab.tmp"), b"").unwrap();
        fs::write(dir.path().join("state.json"), b"{}").unwrap();
        plugins.sweep();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
