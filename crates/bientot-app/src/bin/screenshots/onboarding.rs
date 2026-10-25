//! Each step of the onboarding, `onboarding-<step>[-<state>].png`.

use std::path::PathBuf;
use std::sync::Arc;

use bientot_app::onboarding::Onboarding;
use bientot_app::onboarding_view::{self, OnboardingView};
use bientot_app::settings::data::{Action, Chosen, Refusal};
use bientot_app::strings::Language;
use bientot_store::data_directory::DataDirectory;
use bientot_store::transfer::Contents;
use bientot_windows::startup::StartupRegistrationState;
use gpui::{AppContext as _, HeadlessAppContext, size};
use jiff::civil::date;

use super::capture;
use super::fixtures::SettingsState;

pub fn capture_steps(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for shot in OnboardingShot::ALL {
        capture_onboarding(cx, language, shot, output)?;
    }
    Ok(())
}

/// The pages of the onboarding.
#[derive(Clone, Copy)]
enum OnboardingShot {
    Language,
    /// « Importer des données… » clicked: the file dialog runs, or the file is read.
    LanguageImportReading,
    /// The file exported on the previous PC, read and waiting for « Importer et redémarrer ».
    LanguageImportConfirm,
    /// A file that is no Bientôt export.
    LanguageImportRefused,
    Schedule,
    /// The daily target typed as « 7h », then « Suivant ».
    ScheduleInvalid,
    Connections,
    /// « Voir ce qui est envoyé » opened, as a click does.
    ConnectionsDetails,
    /// The ZIP build, which leaves the updates out.
    ConnectionsZip,
    /// « Commencer » pressed, and the settings not saved.
    ConnectionsSaveFailed,
}

impl OnboardingShot {
    const ALL: [OnboardingShot; 10] = [
        Self::Language,
        Self::LanguageImportReading,
        Self::LanguageImportConfirm,
        Self::LanguageImportRefused,
        Self::Schedule,
        Self::ScheduleInvalid,
        Self::Connections,
        Self::ConnectionsDetails,
        Self::ConnectionsZip,
        Self::ConnectionsSaveFailed,
    ];

    fn slug(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::LanguageImportReading => "language-import-reading",
            Self::LanguageImportConfirm => "language-import-confirm",
            Self::LanguageImportRefused => "language-import-refused",
            Self::Schedule => "schedule",
            Self::ScheduleInvalid => "schedule-invalid",
            Self::Connections => "connections",
            Self::ConnectionsDetails => "connections-details",
            Self::ConnectionsZip => "connections-zip",
            Self::ConnectionsSaveFailed => "connections-save-failed",
        }
    }

    fn apply(self, view: &mut OnboardingView, cx: &mut gpui::Context<OnboardingView>) {
        view.onboarding_mut()
            .show_startup(StartupRegistrationState::Enabled);
        let steps = match self {
            Self::Language
            | Self::LanguageImportReading
            | Self::LanguageImportConfirm
            | Self::LanguageImportRefused => 0,
            Self::Schedule | Self::ScheduleInvalid => 1,
            Self::Connections
            | Self::ConnectionsDetails
            | Self::ConnectionsZip
            | Self::ConnectionsSaveFailed => 2,
        };
        for _ in 0..steps {
            view.onboarding_mut().advance();
        }
        let importing = matches!(
            self,
            Self::LanguageImportReading | Self::LanguageImportConfirm | Self::LanguageImportRefused
        );
        if importing && let Some(page) = view.onboarding_mut().import_mut() {
            page.start(Action::Import);
        }
        match self {
            Self::LanguageImportConfirm => view.read_import(Some(Ok(Chosen {
                file: PathBuf::from(r"D:\Bientot-2026-10-21.bientot"),
                contents: Contents {
                    exported_at: "2026-10-21T16:04:00Z".parse().unwrap(),
                    days: Some((date(2025, 9, 1), date(2026, 10, 21))),
                },
                exported_on: date(2026, 10, 21),
                this_pc: None,
            }))),
            Self::LanguageImportRefused => view.read_import(Some(Err(Refusal::NotATransfer))),
            _ => {}
        }
        if let Self::ScheduleInvalid = self {
            view.type_target("7h", cx);
            view.onboarding_mut().advance();
        }
        if let Self::ConnectionsDetails = self {
            view.toggle_details();
        }
        if let Self::ConnectionsSaveFailed = self {
            view.onboarding_mut().advance();
            view.onboarding_mut().finished(false);
        }
    }
}

fn capture_onboarding(
    cx: &mut HeadlessAppContext,
    language: Language,
    shot: OnboardingShot,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let window = cx.open_window(
        size(
            onboarding_view::WINDOW_WIDTH,
            onboarding_view::WINDOW_HEIGHT,
        ),
        |window, cx| {
            let mut usage_ping = SettingsState::NeverSent.usage_ping();
            if let OnboardingShot::ConnectionsZip = shot {
                usage_ping.build = SettingsState::Zip.usage_ping().build;
            }
            let directory =
                DataDirectory::from(PathBuf::from(r"C:\Users\Camille\AppData\Local\Bientot"));
            let onboarding = Onboarding::new(
                SettingsState::Saved.settings(),
                usage_ping,
                Some(language),
                Some(directory),
            );
            cx.new(|cx| {
                OnboardingView::new(
                    onboarding,
                    Arc::new(|_| Ok(())),
                    Box::new(|_, _| {}),
                    window,
                    cx,
                )
            })
        },
    )?;
    cx.update(|cx| {
        window.update(cx, |view, _, cx| {
            shot.apply(view, cx);
            cx.notify();
        })
    })?;
    let path = output.join(format!("onboarding-{}.png", shot.slug()));
    capture(cx, window.into(), &path)
}
