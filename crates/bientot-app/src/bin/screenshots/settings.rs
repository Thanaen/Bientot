//! « Paramètres » on each page and in each state, `settings-<page>[-<state>].png`, and the
//! entrances `settings-schedule-unsaved-motion-<0|7|13|50|100>.png` and
//! `settings-general-language-open-motion-<0|8|17|50|100>.png`, and « Données » in each state,
//! `settings-data[-<state>].png`, and « Plugins » in each state, `settings-plugins[-<state>].png`.

use std::path::PathBuf;

use anyhow::Context as _;
use bientot_app::plugins::{Consent, Installed, Offered, Refused};
use bientot_app::settings::data::{Action, Backup, Chosen, DataPage, Exported, Refusal};
use bientot_app::settings::plugins::PluginsPage;
use bientot_app::settings::{Page, SettingsForm};
use bientot_app::settings_view::{self, SettingsView};
use bientot_app::strings::Language;
use bientot_app::theme::motion;
use bientot_domain::configuration::Settings;
use bientot_plugin_api::{Capability, PluginId};
use bientot_plugins::state::Status;
use bientot_store::data_directory::DataDirectory;
use bientot_store::settings::SettingsStore;
use bientot_store::transfer::Contents;
use bientot_windows::startup::StartupRegistrationState;
use gpui::{AppContext as _, HeadlessAppContext, size};
use jiff::civil::date;

use super::fixtures::SettingsState;
use super::{capture, capture_entrance};

pub fn capture_pages(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for page in Page::ALL {
        capture_settings(cx, language, page, SettingsState::Saved, output)?;
    }
    for state in [
        SettingsState::Saving,
        SettingsState::FieldError,
        SettingsState::SaveFailed,
        SettingsState::Unsaved,
    ] {
        capture_settings(cx, language, Page::Schedule, state, output)?;
    }
    capture_settings(
        cx,
        language,
        Page::Notifications,
        SettingsState::DepartureOff,
        output,
    )?;
    for state in [
        SettingsState::ActivityWatch,
        SettingsState::DetailsOpen,
        SettingsState::UsageOff,
        SettingsState::NeverSent,
        SettingsState::Startup(StartupRegistrationState::DisabledByUser),
        SettingsState::Startup(StartupRegistrationState::DisabledByPolicy),
        SettingsState::Startup(StartupRegistrationState::EnabledByPolicy),
        SettingsState::Startup(StartupRegistrationState::RegistrationMismatch),
        SettingsState::Startup(StartupRegistrationState::Unavailable),
        SettingsState::StartupRefused,
        SettingsState::StartupOtherCopy,
        SettingsState::Zip,
        SettingsState::ZipDetailsOpen,
        SettingsState::End,
        SettingsState::DirectDetailsOpen,
        SettingsState::LanguageOpen,
    ] {
        capture_settings(cx, language, Page::General, state, output)?;
    }
    for state in [
        SettingsState::Appointments,
        SettingsState::AppointmentNew,
        SettingsState::AppointmentEdit,
        SettingsState::AppointmentHoursError,
        SettingsState::AppointmentDeleteConfirm,
        SettingsState::AppointmentAlternate,
    ] {
        capture_settings(cx, language, Page::Accounting, state, output)?;
    }
    for state in [
        SettingsState::Saved,
        SettingsState::End,
        SettingsState::Wrapped,
    ] {
        capture_settings(cx, language, Page::Json, state, output)?;
    }
    for state in DataState::ALL {
        capture_data(cx, language, state, output)?;
    }
    Ok(())
}

/// « Plugins » in each state.
pub fn capture_plugin_pages(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for state in PluginsState::ALL {
        capture_plugins(cx, language, state, output)?;
    }
    Ok(())
}

/// The states of « Plugins ».
#[derive(Clone, Copy)]
enum PluginsState {
    /// The four sample plugins, on, off, stopped after errors and with files changed, and one
    /// blocked by Bientôt.
    List,
    Empty,
    /// A package for an installed plugin, signed by another author.
    OtherAuthor,
    Installed,
    Consent,
    /// Unsigned, and asking for more memory than the default.
    ConsentUnsigned,
}

impl PluginsState {
    const ALL: [PluginsState; 6] = [
        Self::List,
        Self::Empty,
        Self::OtherAuthor,
        Self::Installed,
        Self::Consent,
        Self::ConsentUnsigned,
    ];

    fn slug(self) -> Option<&'static str> {
        match self {
            Self::List => None,
            Self::Empty => Some("empty"),
            Self::OtherAuthor => Some("other-author"),
            Self::Installed => Some("installed"),
            Self::Consent => Some("consent"),
            Self::ConsentUnsigned => Some("consent-unsigned"),
        }
    }

    fn page(self, language: Language) -> PluginsPage {
        let mut page = PluginsPage::new(language);
        let consent = |fingerprint: Option<&str>, memory_mib| Consent {
            name: name(language, "Focus", "Concentration"),
            version: "0.1.0".into(),
            replaces: None,
            fingerprint: fingerprint.map(Into::into),
            capabilities: vec![Capability::ReadDay, Capability::UiFlyout],
            memory_mib,
            needs_consent: true,
        };
        match self {
            Self::List | Self::Empty => {}
            Self::OtherAuthor => {
                page.start();
                page.read(Err(Refused::OtherAuthor {
                    bound: FINGERPRINT.into(),
                    offered: "0c4e 9a7f 31d2 b85e 6f10 a4c9 d273 58e1".into(),
                }));
            }
            Self::Installed => page.installed(consent(Some(FINGERPRINT), None), true),
            Self::Consent | Self::ConsentUnsigned => {
                let consent = match self {
                    Self::Consent => consent(Some(FINGERPRINT), None),
                    _ => consent(None, Some(16)),
                };
                page.start();
                page.read(Ok(Offered::preview(consent)));
            }
        }
        page
    }

    fn installed(self, language: Language) -> Vec<Installed> {
        if let Self::Empty = self {
            return Vec::new();
        }
        let plugin =
            |id: &str, (en, fr), fingerprint: Option<&str>, capabilities, status| Installed {
                id: PluginId::parse(id).unwrap(),
                name: name(language, en, fr),
                version: "0.1.0".into(),
                fingerprint: fingerprint.map(Into::into),
                capabilities,
                status,
            };
        let both = || vec![Capability::ReadDay, Capability::UiFlyout];
        vec![
            plugin(
                "bientot.focus",
                ("Focus", "Concentration"),
                Some(FINGERPRINT),
                both(),
                Status::Active,
            ),
            plugin(
                "bientot.tomato",
                ("Tomato", "Tomate"),
                Some(FINGERPRINT),
                vec![Capability::UiFlyout],
                Status::Off,
            ),
            plugin(
                "bientot.battery",
                ("Battery", "Batterie"),
                None,
                both(),
                Status::FailedUntilRestart {
                    reason: "trap".into(),
                },
            ),
            plugin(
                "bientot.weekend",
                ("Weekend", "Vivement le week-end"),
                None,
                both(),
                Status::FilesChanged,
            ),
            plugin(
                "someone.weather",
                ("Weather", "Météo"),
                Some("0c4e 9a7f 31d2 b85e 6f10 a4c9 d273 58e1"),
                both(),
                Status::Revoked {
                    reason_en: "It sent your day to a server.".into(),
                    reason_fr: "Il envoyait votre journée à un serveur.".into(),
                },
            ),
        ]
    }
}

/// An author's fingerprint, for the screenshots.
const FINGERPRINT: &str = "7f3a 21c9 e04b 88d1 5c62 b9a0 13fe 4d77";

fn name(language: Language, en: &str, fr: &str) -> String {
    match language {
        Language::French => fr,
        Language::English => en,
    }
    .to_owned()
}

fn capture_plugins(
    cx: &mut HeadlessAppContext,
    language: Language,
    state: PluginsState,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let settings = SettingsState::Saved.settings();
    let file = settings_file(&settings)?;
    let window = cx.open_window(
        size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT),
        |window, cx| {
            let mut form = SettingsForm::new(settings, SettingsState::Saved.usage_ping(), language);
            form.go_to(Page::Plugins);
            cx.new(|cx| {
                let mut view = SettingsView::new(form, Some(file), window, cx);
                view.show_plugins(state.page(language), state.installed(language));
                view
            })
        },
    )?;
    let name = match state.slug() {
        Some(state) => format!("settings-plugins-{state}"),
        None => "settings-plugins".to_owned(),
    };
    cx.run_until_parked();
    cx.advance_clock(motion::DIALOG.duration());
    capture(cx, window.into(), &output.join(format!("{name}.png")))?;
    std::fs::remove_dir_all(screenshot_root()).ok();
    Ok(())
}

/// The states of « Données ».
#[derive(Clone, Copy)]
enum DataState {
    Ready,
    Exporting,
    Exported,
    NotATransfer,
    NewerVersion,
    Confirm,
    ConfirmLoss,
    ConfirmEmpty,
    Restarting,
    ImportFailed,
    Backup,
    RestoreConfirm,
    RestoreUnreadable,
    Demo,
}

impl DataState {
    const ALL: [Self; 14] = [
        Self::Ready,
        Self::Exporting,
        Self::Exported,
        Self::NotATransfer,
        Self::NewerVersion,
        Self::Confirm,
        Self::ConfirmLoss,
        Self::ConfirmEmpty,
        Self::Restarting,
        Self::ImportFailed,
        Self::Backup,
        Self::RestoreConfirm,
        Self::RestoreUnreadable,
        Self::Demo,
    ];

    fn slug(self) -> Option<&'static str> {
        Some(match self {
            Self::Ready => return None,
            Self::Exporting => "exporting",
            Self::Exported => "exported",
            Self::NotATransfer => "not-a-transfer",
            Self::NewerVersion => "newer-version",
            Self::Confirm => "confirm",
            Self::ConfirmLoss => "confirm-loss",
            Self::ConfirmEmpty => "confirm-empty",
            Self::Restarting => "restarting",
            Self::ImportFailed => "import-failed",
            Self::Backup => "backup",
            Self::RestoreConfirm => "restore-confirm",
            Self::RestoreUnreadable => "restore-unreadable",
            Self::Demo => "demo",
        })
    }

    fn page(self, language: Language) -> DataPage {
        let backup_file = PathBuf::from(
            r"C:\Users\Camille\AppData\Local\Bientot\backups\before-import-2026-10-22-11-30-00.bientot",
        );
        let backup = matches!(
            self,
            Self::Backup | Self::RestoreConfirm | Self::RestoreUnreadable
        )
        .then(|| Backup {
            file: backup_file.clone(),
            saved_on: date(2026, 10, 22),
        });
        let directory = (!matches!(self, Self::Demo))
            .then(|| DataDirectory::from(PathBuf::from(r"C:\Users\Camille\AppData\Local\Bientot")));
        let mut page = DataPage::new(language, directory, backup);
        let confirm = |page: &mut DataPage, days, this_pc| {
            page.start(Action::Import);
            page.read(Some(Ok(Chosen {
                file: PathBuf::from(r"D:\Bientot-2026-10-21.bientot"),
                contents: Contents {
                    exported_at: "2026-10-21T16:04:00Z".parse().unwrap(),
                    days,
                },
                exported_on: date(2026, 10, 21),
                this_pc,
            })));
        };
        let year = Some((date(2025, 9, 1), date(2026, 10, 21)));
        match self {
            Self::Ready | Self::Demo | Self::Backup => {}
            Self::Exporting => {
                page.start(Action::Export);
            }
            Self::Exported => {
                page.start(Action::Export);
                page.exported(Some(Exported::Saved(PathBuf::from(
                    "D:/Bientot-2026-10-22.bientot",
                ))));
            }
            Self::NotATransfer | Self::NewerVersion => {
                page.start(Action::Import);
                page.read(Some(Err(match self {
                    Self::NotATransfer => Refusal::NotATransfer,
                    _ => Refusal::NewerVersion,
                })));
            }
            Self::Confirm => confirm(&mut page, year, None),
            // An older export, on the PC that kept recording since.
            Self::ConfirmLoss => confirm(
                &mut page,
                Some((date(2025, 9, 1), date(2026, 9, 30))),
                Some((date(2025, 9, 1), date(2026, 10, 21))),
            ),
            Self::ConfirmEmpty => confirm(&mut page, None, year),
            // The copy of the data an import of an older export replaced.
            Self::RestoreConfirm => {
                page.start(Action::Restore);
                page.read(Some(Ok(Chosen {
                    file: backup_file,
                    contents: Contents {
                        exported_at: "2026-10-22T09:30:00Z".parse().unwrap(),
                        days: Some((date(2025, 9, 1), date(2026, 10, 22))),
                    },
                    exported_on: date(2026, 10, 22),
                    this_pc: Some((date(2025, 9, 1), date(2026, 9, 30))),
                })));
            }
            Self::RestoreUnreadable => {
                page.start(Action::Restore);
                page.read(Some(Err(Refusal::Unreadable)));
            }
            Self::Restarting | Self::ImportFailed => {
                confirm(&mut page, year, None);
                page.confirm();
                if let Self::ImportFailed = self {
                    page.import_failed();
                }
            }
        }
        page
    }
}

fn capture_data(
    cx: &mut HeadlessAppContext,
    language: Language,
    state: DataState,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let settings = SettingsState::Saved.settings();
    let file = settings_file(&settings)?;
    let window = cx.open_window(
        size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT),
        |window, cx| {
            let mut form = SettingsForm::new(settings, SettingsState::Saved.usage_ping(), language);
            form.go_to(Page::Data);
            cx.new(|cx| {
                let mut view = SettingsView::new(form, Some(file), window, cx);
                view.show_data(state.page(language));
                view
            })
        },
    )?;
    let name = match state.slug() {
        Some(state) => format!("settings-data-{state}"),
        None => "settings-data".to_owned(),
    };
    // The dialog's entrance, run to its end.
    cx.run_until_parked();
    cx.advance_clock(motion::DIALOG.duration());
    capture(cx, window.into(), &output.join(format!("{name}.png")))?;
    std::fs::remove_dir_all(screenshot_root()).ok();
    Ok(())
}

/// `settings` saved under a fixed folder, which the header names as Windows would.
fn settings_file(settings: &Settings) -> anyhow::Result<PathBuf> {
    let path = screenshot_root()
        .join("Users/Camille/AppData/Local/Packages")
        .join("Thanaen.Bientt_j4qejy376e90w/LocalCache/Local/Bientot/settings.json");
    SettingsStore::new(&path)?.save(settings)?;
    Ok(path)
}

/// `file` with its `notifications` member on one line, as a hand edit may leave it.
fn one_line_notifications(file: &std::path::Path) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(file)?;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("\"notifications\""))
        .context("the settings name their notifications")?;
    let end = start
        + lines[start..]
            .iter()
            .skip(1)
            .position(|line| indent(line) == indent(&lines[start]))
            .context("the notifications close")?
        + 1;
    let joined: Vec<&str> = lines[start..=end].iter().map(|line| line.trim()).collect();
    let line = format!(
        "{}{}",
        &lines[start][..indent(&lines[start])],
        joined.join(" ")
    );
    lines.splice(start..=end, [line]);
    std::fs::write(file, lines.join("\n") + "\n")?;
    Ok(())
}

fn screenshot_root() -> PathBuf {
    std::env::temp_dir().join("WorkTimeScreenshots")
}

fn capture_settings(
    cx: &mut HeadlessAppContext,
    language: Language,
    page: Page,
    state: SettingsState,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let file = settings_file(&state.settings())?;
    if let SettingsState::Wrapped = state {
        one_line_notifications(&file)?;
    }
    let window = cx.open_window(
        size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT),
        |window, cx| {
            let mut form = SettingsForm::new(state.settings(), state.usage_ping(), language);
            form.go_to(page);
            cx.new(|cx| {
                let mut view = SettingsView::new(form, Some(file), window, cx);
                view.show_folder_as(r"C:\Users\Camille\AppData\Local\Bientot");
                view
            })
        },
    )?;
    cx.update(|cx| {
        window.update(cx, |view, window, cx| {
            state.apply(view, window, cx);
            cx.notify();
        })
    })?;
    let page = match page {
        Page::General => "general",
        Page::Schedule => "schedule",
        Page::Accounting => "accounting",
        Page::Notifications => "notifications",
        Page::Plugins => "plugins",
        Page::Data => "data",
        Page::Json => "json",
    };
    let name = match state.slug() {
        Some(state) => format!("settings-{page}-{state}"),
        None => format!("settings-{page}"),
    };
    if let Some((timing, frames)) = state.entrance() {
        capture_entrance(cx, window.into(), timing, frames, output, &name)?;
    }
    capture(cx, window.into(), &output.join(format!("{name}.png")))?;
    std::fs::remove_dir_all(screenshot_root()).ok();
    Ok(())
}
