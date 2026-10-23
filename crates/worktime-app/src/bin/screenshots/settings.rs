//! « Paramètres » on each page and in each state, `settings-<page>[-<state>].png`, and the
//! entrances `settings-schedule-unsaved-motion-<0|7|13|50|100>.png` and
//! `settings-general-language-open-motion-<0|8|17|50|100>.png`, and « Données » in each state,
//! `settings-data[-<state>].png`.

use std::path::PathBuf;

use anyhow::Context as _;
use gpui::{AppContext as _, HeadlessAppContext, size};
use jiff::civil::date;
use worktime_app::settings::data::{Action, Backup, Chosen, DataPage, Exported, Refusal};
use worktime_app::settings::{Page, SettingsForm};
use worktime_app::settings_view::{self, SettingsView};
use worktime_app::strings::Language;
use worktime_app::theme::motion;
use worktime_domain::configuration::WorkTimeSettings;
use worktime_store::data_directory::DataDirectory;
use worktime_store::settings::SettingsStore;
use worktime_store::transfer::Contents;
use worktime_windows::startup::StartupRegistrationState;

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
    Demo,
}

impl DataState {
    const ALL: [Self; 12] = [
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
            Self::Demo => "demo",
        })
    }

    fn page(self, language: Language) -> DataPage {
        let backup = matches!(self, Self::Backup).then(|| Backup {
            file: PathBuf::from(
                r"C:\Users\Camille\AppData\Local\WorkTime\backups\before-import-2026-10-22-11-30-00.worktime",
            ),
            saved_on: date(2026, 10, 22),
        });
        let directory = (!matches!(self, Self::Demo)).then(|| {
            DataDirectory::from(PathBuf::from(r"C:\Users\Camille\AppData\Local\WorkTime"))
        });
        let mut page = DataPage::new(language, directory, backup);
        let confirm = |page: &mut DataPage, days, this_pc| {
            page.start(Action::Import);
            page.read(Some(Ok(Chosen {
                file: PathBuf::from(r"D:\WorkTime-2026-10-21.worktime"),
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
                    "D:/WorkTime-2026-10-22.worktime",
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
fn settings_file(settings: &WorkTimeSettings) -> anyhow::Result<PathBuf> {
    let path = screenshot_root()
        .join("Users/Camille/AppData/Local/Packages")
        .join("Thanaen.WorkTime_8wekyb3d8bbwe/LocalCache/Local/WorkTime/settings.json");
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
                view.show_folder_as(r"C:\Users\Camille\AppData\Local\WorkTime");
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
