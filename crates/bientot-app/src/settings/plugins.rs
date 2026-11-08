//! The « Plugins » page (`specs/003-plugin-runtime`, FR-044): « Installer depuis un fichier… »
//! and the plugins folder, then the installed plugins with their version, author,
//! capabilities and state, a switch and « Retirer » for each. A package that asks for something
//! shows the consent dialog first (FR-007). Nothing here knows GPUI; [`crate::plugins`] does
//! the work. Every text is the host's: a plugin only gives its name (FR-036).

use bientot_plugin_api::PluginId;
use bientot_plugins::state::Status;

use crate::plugins::{Consent, Installed, Offered, Refused, plugin_language};
use crate::strings::{Key, Language};

/// What the page is doing.
#[derive(Debug)]
enum Step {
    Idle,
    /// The open dialog runs, or the chosen package is read and checked.
    Reading,
    /// The consent dialog asks before the install.
    Confirming(Box<Offered>),
}

/// How the last action ended, until the next one starts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ending {
    Installed(Consent),
    InstallFailed,
    Refused(Refused),
    Removed(String),
    RemoveFailed(String),
}

pub struct PluginsPage {
    language: Language,
    step: Step,
    ending: Option<Ending>,
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginsView {
    pub title: &'static str,
    /// Under the title: what runs, or how the last removal went.
    pub status: Option<String>,
    pub busy: bool,
    pub installed_heading: &'static str,
    pub rows: Vec<PluginRow>,
    /// In place of the rows when there are none.
    pub empty: Option<&'static str>,
    pub add_heading: &'static str,
    pub install: AddRow,
    pub folder: AddRow,
}

/// An installed plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRow {
    pub id: PluginId,
    pub name: String,
    /// Its version and its author.
    pub description: String,
    /// What it may do, in the host's words.
    pub capabilities: String,
    /// Why it does not run, when it cannot be turned on.
    pub problem: Option<Problem>,
    /// On as the user left it, even while stopped after errors.
    pub on: bool,
    /// Only a plugin on or turned off by the user has a switch that acts.
    pub switchable: bool,
    /// The switch's accessible name: « Activer {name} ».
    pub switch_name: String,
    pub remove: &'static str,
    pub remove_name: String,
}

/// Why an installed plugin does not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub text: String,
    /// It stays off until the user acts: removed and installed again, or updated. A stop
    /// after errors ends by itself at the next start.
    pub blocking: bool,
}

/// « Installer un plugin » or « Dossier des plugins ».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddRow {
    pub title: &'static str,
    pub description: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    /// How the row's last action went.
    pub status: Option<String>,
    pub error: Option<String>,
}

/// The dialog that asks before a plugin is installed (FR-007).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentDialog {
    pub title: String,
    /// Its version, then its author's fingerprint, on a line of its own, when signed.
    pub paragraphs: Vec<String>,
    pub intro: &'static str,
    /// Each capability in plain words, then the memory above the default.
    pub list: Vec<String>,
    /// An unsigned package: its author cannot be verified and it never updates.
    pub warning: Option<&'static str>,
    pub message: &'static str,
    pub cancel: &'static str,
    pub confirm: &'static str,
}

impl PluginsPage {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            step: Step::Idle,
            ending: None,
        }
    }

    /// « Installer depuis un fichier… », or a file dropped on the page. Returns whether the
    /// read may start: one action at a time.
    pub fn start(&mut self) -> bool {
        if !matches!(self.step, Step::Idle) {
            return false;
        }
        self.step = Step::Reading;
        self.ending = None;
        true
    }

    /// The open dialog closed without a file.
    pub fn dismissed(&mut self) {
        if matches!(self.step, Step::Reading) {
            self.step = Step::Idle;
        }
    }

    /// The package was read. Returns it when it installs without asking: an update by the same
    /// author that asks for nothing more (FR-006).
    pub fn read(&mut self, read: Result<Offered, Refused>) -> Option<Offered> {
        if !matches!(self.step, Step::Reading) {
            return None;
        }
        self.step = Step::Idle;
        match read {
            Err(refused) => {
                self.ending = Some(Ending::Refused(refused));
                None
            }
            Ok(offered) if offered.consent.needs_consent => {
                self.step = Step::Confirming(Box::new(offered));
                None
            }
            Ok(offered) => Some(offered),
        }
    }

    /// « Installer » in the consent dialog.
    pub fn confirm(&mut self) -> Option<Offered> {
        match std::mem::replace(&mut self.step, Step::Idle) {
            Step::Confirming(offered) => Some(*offered),
            step => {
                self.step = step;
                None
            }
        }
    }

    /// « Annuler », or Escape: nothing is installed.
    pub fn cancel(&mut self) {
        if matches!(self.step, Step::Confirming(_)) {
            self.step = Step::Idle;
        }
    }

    /// The install of `consent`'s package ended.
    pub fn installed(&mut self, consent: Consent, ok: bool) {
        self.ending = Some(if ok {
            Ending::Installed(consent)
        } else {
            Ending::InstallFailed
        });
    }

    /// « Retirer » ended for the plugin called `name`.
    pub fn removed(&mut self, name: String, ok: bool) {
        self.ending = Some(if ok {
            Ending::Removed(name)
        } else {
            Ending::RemoveFailed(name)
        });
    }

    pub fn dialog(&self) -> Option<ConsentDialog> {
        match &self.step {
            Step::Confirming(offered) => Some(consent_dialog(&offered.consent, self.language)),
            _ => None,
        }
    }

    /// The page, with the plugins `installed` lists: `None` where there are none to have, as in
    /// a demo.
    pub fn view(&self, installed: Option<&[Installed]>) -> PluginsView {
        let language = self.language;
        let available = installed.is_some();
        let installed = installed.unwrap_or_default();
        let reading = matches!(self.step, Step::Reading);
        let (status, install_status, install_error) = match &self.ending {
            None => (None, None, None),
            Some(Ending::Installed(consent)) => {
                let text = match &consent.replaces {
                    Some(_) => language.format(
                        Key::SettingsPluginsUpdatedFormat,
                        &[&consent.name, &consent.version],
                    ),
                    None => language.format(Key::SettingsPluginsInstalledFormat, &[&consent.name]),
                };
                (None, Some(text), None)
            }
            Some(Ending::InstallFailed) => (
                None,
                None,
                Some(language.text(Key::SettingsPluginsInstallFailed).to_owned()),
            ),
            Some(Ending::Refused(refused)) => (None, None, Some(refusal(refused, language))),
            Some(Ending::Removed(name)) => (
                Some(language.format(Key::SettingsPluginsRemovedFormat, &[name])),
                None,
                None,
            ),
            Some(Ending::RemoveFailed(name)) => (
                Some(language.format(Key::SettingsPluginsRemoveFailedFormat, &[name])),
                None,
                None,
            ),
        };
        PluginsView {
            title: language.text(Key::SettingsPluginsTitle),
            status: if reading {
                Some(language.text(Key::SettingsPluginsReading).to_owned())
            } else if !available {
                Some(language.text(Key::SettingsPluginsUnavailable).to_owned())
            } else {
                status
            },
            busy: reading,
            installed_heading: language.text(Key::SettingsPluginsInstalledGroup),
            rows: installed
                .iter()
                .map(|plugin| row(plugin, language))
                .collect(),
            empty: installed
                .is_empty()
                .then(|| language.text(Key::SettingsPluginsEmpty)),
            add_heading: language.text(Key::SettingsPluginsAddGroup),
            install: AddRow {
                title: language.text(Key::SettingsPluginsInstallTitle),
                description: language.text(Key::SettingsPluginsInstallDescription),
                label: language.text(Key::SettingsPluginsInstallButton),
                enabled: available && !reading,
                status: install_status,
                error: install_error,
            },
            folder: AddRow {
                title: language.text(Key::SettingsPluginsFolderTitle),
                description: language.text(Key::SettingsPluginsFolderDescription),
                label: language.text(Key::SettingsPluginsFolderButton),
                enabled: available,
                status: None,
                error: None,
            },
        }
    }
}

fn row(plugin: &Installed, language: Language) -> PluginRow {
    let version = language.format(Key::SettingsPluginsVersionFormat, &[&plugin.version]);
    let author = match &plugin.fingerprint {
        Some(fingerprint) => language.format(Key::SettingsPluginsFingerprintFormat, &[fingerprint]),
        None => language.text(Key::SettingsPluginsUnsigned).to_owned(),
    };
    let blocking = |text: String| Problem {
        text,
        blocking: true,
    };
    let problem = match &plugin.status {
        Status::Active | Status::Off => None,
        Status::FailedUntilRestart { .. } => Some(Problem {
            text: language.text(Key::SettingsPluginsFailed).into(),
            blocking: false,
        }),
        Status::Revoked {
            reason_en,
            reason_fr,
        } => Some(blocking(language.format(
            Key::SettingsPluginsRevokedFormat,
            &[match language {
                Language::French => reason_fr,
                Language::English => reason_en,
            }],
        ))),
        Status::FilesChanged => Some(blocking(
            language.text(Key::SettingsPluginsFilesChanged).into(),
        )),
        Status::UnsupportedApi => Some(blocking(
            language.text(Key::SettingsPluginsUnsupportedApi).into(),
        )),
    };
    PluginRow {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        description: format!("{version} · {author}"),
        capabilities: plugin
            .capabilities
            .iter()
            .map(|capability| capability.consent(plugin_language(language)))
            .collect::<Vec<_>>()
            .join(" · "),
        problem,
        on: matches!(
            plugin.status,
            Status::Active | Status::FailedUntilRestart { .. }
        ),
        switchable: matches!(plugin.status, Status::Active | Status::Off),
        switch_name: language.format(Key::SettingsPluginsTurnOnFormat, &[&plugin.name]),
        remove: language.text(Key::SettingsPluginsRemove),
        remove_name: language.format(Key::SettingsPluginsRemoveFormat, &[&plugin.name]),
    }
}

fn consent_dialog(consent: &Consent, language: Language) -> ConsentDialog {
    let update = consent.replaces.is_some();
    let mut paragraphs = vec![match &consent.replaces {
        Some(old) => language.format(Key::PluginConsentReplacesFormat, &[&consent.version, old]),
        None => language.format(Key::PluginConsentVersionFormat, &[&consent.version]),
    }];
    paragraphs.extend(
        consent.fingerprint.as_ref().map(|fingerprint| {
            format!("{}\n{fingerprint}", language.text(Key::PluginConsentAuthor))
        }),
    );
    let mut list: Vec<String> = consent
        .capabilities
        .iter()
        .map(|capability| capability.consent(plugin_language(language)).to_owned())
        .collect();
    list.extend(
        consent
            .memory_mib
            .map(|mib| language.format(Key::PluginConsentMemoryFormat, &[&mib])),
    );
    ConsentDialog {
        title: language.format(
            if update {
                Key::PluginConsentUpdateTitleFormat
            } else {
                Key::PluginConsentTitleFormat
            },
            &[&consent.name],
        ),
        paragraphs,
        intro: language.text(Key::PluginConsentIntro),
        list,
        warning: consent
            .fingerprint
            .is_none()
            .then(|| language.text(Key::PluginConsentUnsigned)),
        message: language.text(Key::PluginConsentNothingElse),
        cancel: language.text(Key::PluginConsentCancel),
        confirm: language.text(if update {
            Key::PluginConsentUpdate
        } else {
            Key::PluginConsentInstall
        }),
    }
}

fn refusal(refused: &Refused, language: Language) -> String {
    match refused {
        Refused::Unreadable => language.text(Key::SettingsPluginsUnreadable).to_owned(),
        Refused::Package(message) => message.clone(),
        Refused::Module => language.text(Key::SettingsPluginsModuleRefused).to_owned(),
        Refused::OtherAuthor { bound, offered } => language.format(
            Key::SettingsPluginsOtherAuthorFormat,
            &[&unbroken(bound), &unbroken(offered)],
        ),
        Refused::NowUnsigned { bound } => {
            language.format(Key::SettingsPluginsNowUnsignedFormat, &[&unbroken(bound)])
        }
    }
}

/// A fingerprint that a line never breaks inside: its groups are read together.
fn unbroken(fingerprint: &str) -> String {
    fingerprint.replace(' ', "\u{a0}")
}

#[cfg(test)]
mod tests {
    use bientot_plugin_api::Capability;

    use super::*;

    fn installed(status: Status, fingerprint: Option<&str>) -> Installed {
        Installed {
            id: PluginId::parse("bientot.focus").unwrap(),
            name: "Concentration".into(),
            version: "0.1.0".into(),
            fingerprint: fingerprint.map(Into::into),
            capabilities: vec![Capability::ReadDay, Capability::UiFlyout],
            status,
        }
    }

    fn consent(fingerprint: Option<&str>, replaces: Option<&str>) -> Consent {
        Consent {
            name: "Concentration".into(),
            version: "0.2.0".into(),
            replaces: replaces.map(Into::into),
            fingerprint: fingerprint.map(Into::into),
            capabilities: vec![Capability::ReadDay, Capability::UiFlyout],
            memory_mib: None,
            needs_consent: true,
        }
    }

    #[test]
    fn with_no_plugin_the_page_says_so_and_offers_the_install() {
        let view = PluginsPage::new(Language::French).view(Some(&[]));
        assert_eq!(view.empty, Some("Aucun plugin installé."));
        assert!(view.install.enabled);
        assert_eq!(view.install.label, "Installer depuis un fichier…");
        assert_eq!(view.status, None);
    }

    #[test]
    fn in_a_demo_nothing_can_be_installed() {
        let view = PluginsPage::new(Language::English).view(None);
        assert_eq!(view.status.as_deref(), Some("Plugins aren’t available."));
        assert!(!view.install.enabled && !view.folder.enabled);
    }

    #[test]
    fn a_row_tells_the_version_the_author_and_the_capabilities_in_plain_words() {
        let page = PluginsPage::new(Language::French);
        let row = &page
            .view(Some(&[installed(Status::Active, Some("1a2b 3c4d"))]))
            .rows[0];
        assert_eq!(
            row.description,
            "Version 0.1.0 · Empreinte de l’auteur\u{a0}: 1a2b 3c4d"
        );
        assert_eq!(
            row.capabilities,
            "Lire votre journée en cours · Afficher une carte dans le flyout"
        );
        assert!(row.on && row.switchable);
        assert_eq!(row.problem, None);
        assert_eq!(row.remove_name, "Retirer Concentration");

        let unsigned = &page.view(Some(&[installed(Status::Off, None)])).rows[0];
        assert_eq!(
            unsigned.description,
            "Version 0.1.0 · Non signé\u{a0}: l’auteur n’est pas vérifié"
        );
        assert!(!unsigned.on && unsigned.switchable);
    }

    #[test]
    fn a_plugin_that_cannot_run_says_why_and_its_switch_does_not_act() {
        let page = PluginsPage::new(Language::English);
        // On as the user left it, until the next start.
        for (status, problem, on, blocking) in [
            (
                Status::FailedUntilRestart {
                    reason: "trap".into(),
                },
                "Stopped after three errors. It runs again the next time Bientôt starts.",
                true,
                false,
            ),
            (
                Status::Revoked {
                    reason_en: "It sends your day away.".into(),
                    reason_fr: "Il envoie votre journée ailleurs.".into(),
                },
                "Blocked by Bientôt. It sends your day away.",
                false,
                true,
            ),
            (
                Status::FilesChanged,
                "Its files changed after it was installed. Remove it, then install it again.",
                false,
                true,
            ),
        ] {
            let row = &page.view(Some(&[installed(status, None)])).rows[0];
            assert_eq!(
                row.problem,
                Some(Problem {
                    text: problem.into(),
                    blocking
                })
            );
            assert_eq!((row.on, row.switchable), (on, false));
        }
    }

    #[test]
    fn the_consent_dialog_names_each_capability_the_author_and_the_memory() {
        let mut signed = consent(Some("1a2b 3c4d"), None);
        signed.memory_mib = Some(32);
        let dialog = consent_dialog(&signed, Language::French);
        assert_eq!(dialog.title, "Installer Concentration\u{202f}?");
        assert_eq!(
            dialog.paragraphs,
            ["Version 0.2.0", "Empreinte de l’auteur\u{a0}:\n1a2b 3c4d"]
        );
        assert_eq!(
            dialog.list,
            [
                "Lire votre journée en cours",
                "Afficher une carte dans le flyout",
                "Utiliser jusqu’à 32 Mo de mémoire"
            ]
        );
        assert_eq!(dialog.warning, None);
        assert_eq!(dialog.confirm, "Installer");
    }

    #[test]
    fn an_unsigned_package_warns_that_its_author_cannot_be_verified() {
        let dialog = consent_dialog(&consent(None, Some("0.1.0")), Language::English);
        assert_eq!(dialog.title, "Update Concentration?");
        assert_eq!(dialog.paragraphs, ["Version 0.2.0 (currently 0.1.0)"]);
        assert_eq!(
            dialog.warning,
            Some("The author can’t be verified, and this plugin can’t be\u{a0}updated.")
        );
        assert_eq!(dialog.confirm, "Update");
    }

    #[test]
    fn a_package_by_another_author_is_refused_with_both_fingerprints() {
        let mut page = PluginsPage::new(Language::French);
        assert!(page.start());
        assert!(!page.start(), "one read at a time");
        assert!(page.view(Some(&[])).busy);
        let refused = Refused::OtherAuthor {
            bound: "aaaa bbbb".into(),
            offered: "cccc dddd".into(),
        };
        assert!(page.read(Err(refused)).is_none());
        let view = page.view(Some(&[]));
        assert!(!view.busy && view.install.enabled);
        assert_eq!(
            view.install.error.as_deref(),
            Some(
                "Refusé\u{a0}: seul l’auteur du plugin installé peut le mettre à jour. \
                 Empreinte de l’auteur\u{a0}: aaaa\u{a0}bbbb. Empreinte de ce fichier\u{a0}: \
                 cccc\u{a0}dddd."
            )
        );
        assert!(page.dialog().is_none());
    }

    #[test]
    fn a_package_that_asks_for_something_waits_for_consent_and_an_update_does_not() {
        let mut page = PluginsPage::new(Language::French);
        assert!(page.start());
        assert!(
            page.read(Ok(Offered::preview(consent(None, None))))
                .is_none()
        );
        assert!(page.dialog().is_some());
        assert!(!page.start(), "the dialog holds the page");
        page.cancel();
        assert!(page.dialog().is_none());

        assert!(page.start());
        page.read(Ok(Offered::preview(consent(None, None))));
        let confirmed = page.confirm().expect("the dialog showed");
        assert_eq!(confirmed.consent.name, "Concentration");
        assert!(page.confirm().is_none(), "once");

        let mut update = consent(Some("1a2b"), Some("0.1.0"));
        update.needs_consent = false;
        assert!(page.start());
        assert!(page.read(Ok(Offered::preview(update))).is_some());
        assert!(page.dialog().is_none());
    }

    #[test]
    fn the_install_and_the_removal_say_how_they_went() {
        let mut page = PluginsPage::new(Language::French);
        page.installed(consent(None, None), true);
        assert_eq!(
            page.view(Some(&[])).install.status.as_deref(),
            Some("Concentration est installé.")
        );
        page.installed(consent(None, Some("0.1.0")), true);
        assert_eq!(
            page.view(Some(&[])).install.status.as_deref(),
            Some("Concentration est passé en version 0.2.0.")
        );
        page.removed("Concentration".into(), true);
        let view = page.view(Some(&[]));
        assert_eq!(view.status.as_deref(), Some("Concentration est retiré."));
        assert_eq!(view.install.status, None);
        assert!(page.start(), "a new action clears the last ending");
        assert_eq!(
            page.view(Some(&[])).status.as_deref(),
            Some("Lecture du plugin…")
        );
        page.dismissed();
        assert_eq!(page.view(Some(&[])).status, None);
    }
}
