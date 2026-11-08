//! Installing, loading and removing plugins: the package, its trust, its files on disk and its
//! entry in `state.json` move together. The app calls these on the UI side for an install, then
//! hands the loaded plugins to the `plugins` thread.

use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;

use bientot_plugin_api::{Capability, LocaleTexts, Manifest, ManifestError, PluginId};
use jiff::Timestamp;

use crate::files::PluginsDir;
use crate::package::{self, MANIFEST, MODULE, Package, PackageError};
use crate::runtime::{ModuleError, Runtime};
use crate::state::{Grant, InstalledPlugin, Pending, StateFile, Status};
use crate::thread::PluginSpec;
use crate::trust::{self, Decision, Refusal};

/// A package that passed every check, waiting for the user's consent.
#[derive(Debug)]
pub struct Offer {
    pub package: Package,
    pub decision: Decision,
    /// The installed version this one replaces.
    pub replaces: Option<String>,
}

impl Offer {
    pub fn capabilities(&self) -> &BTreeSet<Capability> {
        &self.package.manifest.capabilities
    }

    /// The author's key fingerprint, or `None` for an unsigned package.
    pub fn fingerprint(&self) -> Option<String> {
        self.package
            .signature
            .as_ref()
            .map(|s| package::fingerprint(&s.key))
    }

    /// The consent dialog is skipped only when the same author ships a manifest that asks for
    /// nothing more (FR-006).
    pub fn needs_consent(&self, state: &StateFile) -> bool {
        let Some(installed) = state.plugins.get(&self.package.manifest.id) else {
            return true;
        };
        !matches!(self.decision, Decision::Update(_))
            || !self
                .package
                .manifest
                .capabilities
                .is_subset(&installed.granted.capabilities)
    }
}

#[derive(Debug)]
pub enum InstallError {
    Package(PackageError),
    Module(ModuleError),
    Trust(Refusal),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Package(e) => e.fmt(f),
            InstallError::Module(e) => e.fmt(f),
            InstallError::Trust(Refusal::OtherAuthor { bound, offered }) => write!(
                f,
                "this package is signed by {offered}, not by {bound}, the author of the installed plugin"
            ),
            InstallError::Trust(Refusal::NowUnsigned { bound }) => write!(
                f,
                "the installed plugin is signed by {bound} and this package is not signed"
            ),
        }
    }
}

impl std::error::Error for InstallError {}

/// Reads and checks a package file: format, manifest, module and trust. Nothing is written.
pub fn offer(
    bytes: &[u8],
    reserved_titles: &[&str],
    runtime: &Runtime,
    state: &StateFile,
) -> Result<Offer, InstallError> {
    let package = Package::read(bytes, reserved_titles).map_err(InstallError::Package)?;
    runtime
        .compile(package.module(), &package.manifest.capabilities)
        .map_err(InstallError::Module)?;
    let installed = state.plugins.get(&package.manifest.id);
    let decision =
        trust::decide(installed.map(|p| &p.trust), &package).map_err(InstallError::Trust)?;
    Ok(Offer {
        replaces: installed.map(|p| p.version.clone()),
        package,
        decision,
    })
}

/// Writes the accepted package and its state entry. An unsigned replacement starts over: its old
/// state goes with it.
pub fn install(dir: &PluginsDir, state: &mut StateFile, offer: Offer) -> io::Result<PluginId> {
    let Offer {
        package,
        decision,
        replaces,
    } = offer;
    dir.install(&package)?;
    let (trust, fresh) = match decision {
        Decision::FirstInstall(t) | Decision::ReplaceUnsigned(t) => (t, true),
        Decision::Update(t) => (t, false),
    };
    let id = package.manifest.id.clone();
    let manifest_hash = package::hex(&package::hash(&package.files[MANIFEST]));
    let previous = if fresh {
        None
    } else {
        state.plugins.remove(&id)
    };
    let pending = match (&previous, replaces) {
        (Some(_), Some(from)) => Some(Pending::Upgraded { from }),
        _ => Some(Pending::Installed),
    };
    let entry = InstalledPlugin {
        version: package.manifest.version.to_string(),
        trust,
        files: package
            .hashes()
            .iter()
            .map(|(path, hash)| (path.clone(), package::hex(hash)))
            .collect(),
        manifest_hash: manifest_hash.clone(),
        granted: Grant {
            capabilities: package.manifest.capabilities.clone(),
            manifest_hash,
        },
        status: Status::Active,
        failures: Vec::new(),
        card_preference: previous.as_ref().is_some_and(|p| p.card_preference),
        hidden_on: previous.and_then(|p| p.hidden_on),
        pending,
    };
    state.plugins.insert(id.clone(), entry);
    state.save(&dir.state_path())?;
    Ok(id)
}

pub fn remove(dir: &PluginsDir, state: &mut StateFile, id: &PluginId) -> io::Result<()> {
    dir.remove(id)?;
    if state.plugins.remove(id).is_some() {
        state.save(&dir.state_path())?;
    }
    Ok(())
}

/// Why an active plugin did not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadProblem {
    FilesChanged,
    UnsupportedApi(ManifestError),
}

/// Checks the files of every active plugin against their hashes and reads them. A plugin whose
/// files changed, or whose manifest this host no longer reads, changes state and stays off.
pub fn load_active(
    dir: &PluginsDir,
    state: &mut StateFile,
    reserved_titles: &[&str],
) -> (Vec<PluginSpec>, Vec<(PluginId, LoadProblem)>) {
    let mut loaded = Vec::new();
    let mut problems = Vec::new();
    for (id, plugin) in &mut state.plugins {
        if !plugin.is_active() {
            continue;
        }
        let spec = match read(dir, id, plugin, reserved_titles) {
            Ok(spec) => spec,
            Err(problem) => {
                match &problem {
                    LoadProblem::FilesChanged => plugin.files_changed(),
                    LoadProblem::UnsupportedApi(_) => plugin.api_unsupported(),
                };
                problems.push((id.clone(), problem));
                continue;
            }
        };
        loaded.push(spec);
    }
    if !problems.is_empty() {
        let _ = state.save(&dir.state_path());
    }
    (loaded, problems)
}

fn read(
    dir: &PluginsDir,
    id: &PluginId,
    plugin: &InstalledPlugin,
    reserved_titles: &[&str],
) -> Result<PluginSpec, LoadProblem> {
    let files = dir
        .load(id, &plugin.files)
        .ok()
        .flatten()
        .ok_or(LoadProblem::FilesChanged)?;
    let manifest = Manifest::parse(&files[MANIFEST], reserved_titles).map_err(|e| {
        if e.problem == bientot_plugin_api::manifest::Problem::UnsupportedApi {
            LoadProblem::UnsupportedApi(e)
        } else {
            LoadProblem::FilesChanged
        }
    })?;
    if manifest.id != *id {
        return Err(LoadProblem::FilesChanged);
    }
    let mut locales = std::collections::BTreeMap::new();
    for tag in &manifest.locales {
        let texts = files
            .get(&format!("locales/{tag}.json"))
            .and_then(|bytes| LocaleTexts::parse(bytes).ok())
            .ok_or(LoadProblem::FilesChanged)?;
        locales.insert(tag.clone(), texts);
    }
    let wasm = files
        .get(MODULE)
        .cloned()
        .ok_or(LoadProblem::FilesChanged)?;
    Ok(PluginSpec {
        manifest: Arc::new(manifest),
        locales: Arc::new(locales),
        wasm: Arc::new(wasm),
        granted: plugin.granted.capabilities.clone(),
        pending: plugin.pending.clone(),
    })
}

/// Records a failure. Returns `true` when it turned the plugin off until the next start.
pub fn record_failure(
    dir: &PluginsDir,
    state: &mut StateFile,
    id: &PluginId,
    at: Timestamp,
    reason: &str,
) -> bool {
    let Some(plugin) = state.plugins.get_mut(id) else {
        return false;
    };
    let off = plugin.record_failure(at, reason);
    let _ = state.save(&dir.state_path());
    off
}

/// `installed` or `upgraded` reached the plugin: it is not sent again.
pub fn delivered(dir: &PluginsDir, state: &mut StateFile, id: &PluginId) {
    if let Some(plugin) = state.plugins.get_mut(id)
        && plugin.pending.take().is_some()
    {
        let _ = state.save(&dir.state_path());
    }
}

#[cfg(test)]
mod tests;
