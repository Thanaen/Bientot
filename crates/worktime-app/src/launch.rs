//! The command line of a launch.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use worktime_core::demo::DemoScenario;
use worktime_store::data_directory::{self, DataDirectory};
use worktime_windows::notifications::{ACTIVATOR_ARGUMENT, COM_SERVER_ARGUMENT};
use worktime_windows::startup::WINDOWS_STARTUP_ARGUMENT;

use crate::strings::Language;
use crate::theme::ThemeMode;
use crate::{demo, onboarding};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// `None` runs on the user's data.
    pub demo: Option<DemoScenario>,
    pub data_dir: Option<PathBuf>,
    pub open_flyout: bool,
    pub theme: ThemeMode,
    /// `None` leaves the language to the settings.
    pub language: Option<Language>,
    /// `--onboarding` shows the onboarding on settings already saved.
    pub onboarding: bool,
    /// `--demo-update` has a demo offer a Store update; it does nothing without a demo.
    pub demo_update: bool,
    /// Set when the start-with-Windows marker ([`WINDOWS_STARTUP_ARGUMENT`]) is present.
    pub windows_startup: bool,
    /// The WorkTime that restarted this one to apply an import ([`RESTARTED_FROM_ARGUMENT`]),
    /// which this one waits for.
    pub restarted_from: Option<u32>,
    pub origin: Origin,
}

/// Names the process id of the WorkTime that restarted this one. An argument, unlike an
/// environment variable, is not handed down to what this WorkTime starts in turn.
pub const RESTARTED_FROM_ARGUMENT: &str = "--restarted-from";

/// What started this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The user, a shortcut or the start-with-Windows registration.
    Command,
    /// COM, to deliver a click on a toast because no running WorkTime registered the toast
    /// activator. The COM server, the manifest's or the portable build's, passes
    /// [`ACTIVATOR_ARGUMENT`] and COM adds
    /// [`COM_SERVER_ARGUMENT`]; either one is enough.
    ToastActivator,
}

/// How a click on a toast reached WorkTime, as `notification.activated` logs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// WorkTime was running.
    Warm,
    /// COM started this process to deliver the click.
    Cold,
}

impl Activation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Warm => "warm",
            Self::Cold => "cold",
        }
    }
}

/// Tells the clicks a process receives apart. COM starts WorkTime for one click, the first it
/// then delivers, and delivers any later one to the running process. The clicks arrive on COM
/// threads, hence the atomic.
#[derive(Debug)]
pub struct Activations {
    cold: AtomicBool,
}

impl Activations {
    pub fn of(origin: Origin) -> Self {
        Self {
            cold: AtomicBool::new(origin == Origin::ToastActivator),
        }
    }

    /// How the click that just arrived reached WorkTime.
    pub fn arrived(&self) -> Activation {
        if self.cold.swap(false, Ordering::Relaxed) {
            Activation::Cold
        } else {
            Activation::Warm
        }
    }
}

impl Launch {
    /// Reads the arguments after the program name, with the .NET app's flags: `--light` and
    /// `--dark` fix the theme, `--french` and `--english` the language, `--data-dir` the data
    /// folder, and `--demo`, `--demo-history-week`, `--demo-problem-periods` and
    /// `--demo-fragmented-day` the demo, `--demo-update` a demo Store update, and `--onboarding`
    /// asks for the onboarding. `--demo` may name a scenario, as the R1 prototype's `--demo late`
    /// did. The start-with-Windows marker sets `windows_startup`, and the toast activator's
    /// arguments set the [`Origin`]. The last flag of a kind wins.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut launch = Self {
            demo: None,
            data_dir: None,
            open_flyout: false,
            theme: ThemeMode::System,
            language: None,
            onboarding: false,
            demo_update: false,
            windows_startup: false,
            restarted_from: None,
            origin: Origin::Command,
        };
        let mut arguments = arguments.into_iter().peekable();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--demo" => {
                    let mut scenario = DemoScenario::OnTrack;
                    if let Some(slug) = arguments.next_if(|next| !next.starts_with("--")) {
                        scenario = demo::from_slug(&slug)
                            .ok_or_else(|| anyhow::anyhow!("unknown demo scenario {slug:?}"))?;
                    }
                    launch.demo = Some(scenario);
                }
                "--demo-history-week" => launch.demo = Some(DemoScenario::HistoryWeek),
                "--demo-problem-periods" => launch.demo = Some(DemoScenario::ProblemPeriods),
                "--demo-fragmented-day" => launch.demo = Some(DemoScenario::FragmentedDay),
                crate::updates::DEMO_ARGUMENT => launch.demo_update = true,
                data_directory::ARGUMENT => {
                    let path = arguments
                        .next()
                        .filter(|path| !path.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("--data-dir needs a folder"))?;
                    launch.data_dir = Some(path.into());
                }
                WINDOWS_STARTUP_ARGUMENT => launch.windows_startup = true,
                RESTARTED_FROM_ARGUMENT => {
                    let id = arguments
                        .next()
                        .and_then(|id| id.parse().ok())
                        .ok_or_else(|| {
                            anyhow::anyhow!("{RESTARTED_FROM_ARGUMENT} needs a process id")
                        })?;
                    launch.restarted_from = Some(id);
                }
                ACTIVATOR_ARGUMENT | COM_SERVER_ARGUMENT => launch.origin = Origin::ToastActivator,
                "--open-flyout" => launch.open_flyout = true,
                onboarding::ARGUMENT => launch.onboarding = true,
                "--light" => launch.theme = ThemeMode::Light,
                "--dark" => launch.theme = ThemeMode::Dark,
                "--french" => launch.language = Some(Language::French),
                "--english" => launch.language = Some(Language::English),
                other => anyhow::bail!("unknown argument {other:?}"),
            }
        }
        Ok(launch)
    }

    /// The folder this launch opens: the `--data-dir` one, else a new throwaway one for a demo,
    /// else the user's, in the folder `find_local_app_data` gives, asked only then.
    pub fn data_directory(
        &self,
        find_local_app_data: impl FnOnce() -> anyhow::Result<PathBuf>,
    ) -> anyhow::Result<DataDirectory> {
        Ok(match (&self.data_dir, self.demo) {
            (Some(path), _) => DataDirectory::named(path)?,
            (None, Some(_)) => DataDirectory::from(demo::throwaway_directory()),
            (None, None) => DataDirectory::user(&find_local_app_data()?),
        })
    }

    /// Whether this launch may send the usage signal at all: only on the user's own folder, never
    /// from a demo nor with `--data-dir`.
    pub fn may_send_usage_ping(&self) -> bool {
        self.demo.is_none() && self.data_dir.is_none()
    }
}

/// The arguments of this launch for the WorkTime a restart starts: never the toast activator's,
/// which no toast would follow, and the start-with-Windows marker only when `windows_startup`
/// keeps it. A restart the user asked for opens the flyout.
pub fn restart_arguments(windows_startup: bool) -> Vec<String> {
    kept_on_restart(std::env::args().skip(1), windows_startup)
}

fn kept_on_restart(
    arguments: impl IntoIterator<Item = String>,
    windows_startup: bool,
) -> Vec<String> {
    let mut kept = Vec::new();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == RESTARTED_FROM_ARGUMENT {
            arguments.next();
        } else if ![ACTIVATOR_ARGUMENT, COM_SERVER_ARGUMENT].contains(&argument.as_str())
            && (windows_startup || argument != WINDOWS_STARTUP_ARGUMENT)
        {
            kept.push(argument);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn parse(arguments: &[&str]) -> anyhow::Result<Launch> {
        Launch::parse(arguments.iter().map(|argument| argument.to_string()))
    }

    #[test]
    fn a_restart_names_the_worktime_it_waits_for_once() {
        assert_eq!(
            parse(&["--restarted-from", "4242", "--dark"])
                .unwrap()
                .restarted_from,
            Some(4242)
        );
        assert!(parse(&["--restarted-from", "me"]).is_err());
        let restarted = |arguments: &[&str]| {
            kept_on_restart(arguments.iter().map(|argument| argument.to_string()), false)
        };
        assert_eq!(
            restarted(&["--restarted-from", "4242", "--data-dir", "D:/wt"]),
            ["--data-dir", "D:/wt"]
        );
    }

    #[test]
    fn defaults_to_the_user_data_and_the_system_theme() {
        assert_eq!(
            parse(&[]).unwrap(),
            Launch {
                demo: None,
                data_dir: None,
                open_flyout: false,
                theme: ThemeMode::System,
                language: None,
                onboarding: false,
                demo_update: false,
                windows_startup: false,
                restarted_from: None,
                origin: Origin::Command,
            }
        );
    }

    #[test]
    fn data_dir_names_the_folder() {
        let launch = parse(&["--data-dir", "C:/Temp/wt"]).unwrap();
        assert_eq!(launch.data_dir, Some(PathBuf::from("C:/Temp/wt")));
        assert_eq!(launch.demo, None);
        assert_eq!(
            parse(&["--data-dir"]).unwrap_err().to_string(),
            "--data-dir needs a folder"
        );
    }

    #[test]
    fn the_startup_marker_marks_a_start_by_the_run_value() {
        assert!(!parse(&[]).unwrap().windows_startup);
        let launch = parse(&["--worktime-windows-startup", "--data-dir", "C:/Temp/wt"]).unwrap();
        assert!(launch.windows_startup);
        assert_eq!(launch.data_dir, Some(PathBuf::from("C:/Temp/wt")));
        assert_eq!(
            parse(&["--WORKTIME-WINDOWS-STARTUP"])
                .unwrap_err()
                .to_string(),
            "unknown argument \"--WORKTIME-WINDOWS-STARTUP\""
        );
    }

    fn directory(arguments: &[&str]) -> PathBuf {
        parse(arguments)
            .unwrap()
            .data_directory(|| Ok(PathBuf::from("/home/ana/.local/share")))
            .unwrap()
            .path()
            .to_owned()
    }

    #[test]
    fn data_dir_opens_the_last_folder_named_made_absolute_demo_or_not() {
        let explicit = std::env::temp_dir().join("WorkTime Data");
        let explicit_text = explicit.to_str().unwrap();
        assert_eq!(directory(&["--data-dir", explicit_text]), explicit);
        assert_eq!(
            directory(&["--demo", "--data-dir", explicit_text]),
            explicit
        );
        assert_eq!(
            directory(&["--data-dir", "portable"]),
            std::env::current_dir().unwrap().join("portable")
        );
        assert_eq!(
            directory(&["--data-dir", "first", "--data-dir", explicit_text]),
            explicit
        );
    }

    #[test]
    fn without_data_dir_a_demo_opens_a_throwaway_folder_and_a_launch_the_users() {
        assert_eq!(directory(&[]), Path::new("/home/ana/.local/share/WorkTime"));
        assert!(
            directory(&["--demo-history-week"])
                .starts_with(std::env::temp_dir().join("WorkTimeDemo"))
        );
    }

    #[test]
    fn does_not_ask_for_local_app_data_unless_it_needs_it() {
        let asked = std::cell::Cell::new(0);
        let unavailable = || -> anyhow::Result<PathBuf> {
            asked.set(asked.get() + 1);
            anyhow::bail!("no folder for the application data")
        };
        let explicit = std::env::temp_dir().join("WorkTime Data");
        for arguments in [&["--data-dir", explicit.to_str().unwrap()][..], &["--demo"]] {
            assert!(
                parse(arguments)
                    .unwrap()
                    .data_directory(unavailable)
                    .is_ok(),
                "{arguments:?}"
            );
        }
        assert_eq!(asked.get(), 0);
        assert_eq!(
            parse(&[])
                .unwrap()
                .data_directory(unavailable)
                .unwrap_err()
                .to_string(),
            "no folder for the application data"
        );
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn only_a_launch_on_the_users_folder_may_send_the_usage_ping() {
        let may_send = |arguments: &[&str]| parse(arguments).unwrap().may_send_usage_ping();
        assert!(may_send(&[]));
        assert!(may_send(&["--worktime-windows-startup", "--dark"]));
        assert!(may_send(&["--toast-activated", "-Embedding"]));
        for arguments in [
            &["--data-dir", "C:\\WorkTime"][..],
            &["--demo"],
            &["--demo", "late"],
            &["--demo-history-week"],
            &["--demo-problem-periods"],
            &["--demo-fragmented-day"],
            &["--data-dir", "--demo"],
            &["--demo", "--data-dir", "C:\\WorkTime"],
        ] {
            assert!(!may_send(arguments), "{arguments:?}");
        }
        assert!(parse(&["--data-dir"]).is_err());
        assert!(parse(&["--notification-smoke"]).is_err());
    }

    #[test]
    fn light_and_dark_fix_the_theme() {
        assert_eq!(parse(&["--light"]).unwrap().theme, ThemeMode::Light);
        assert_eq!(parse(&["--dark"]).unwrap().theme, ThemeMode::Dark);
        assert_eq!(
            parse(&["--light", "--dark"]).unwrap().theme,
            ThemeMode::Dark
        );
    }

    #[test]
    fn french_and_english_fix_the_language() {
        assert_eq!(
            parse(&["--english"]).unwrap().language,
            Some(Language::English)
        );
        assert_eq!(
            parse(&["--english", "--french"]).unwrap().language,
            Some(Language::French)
        );
    }

    #[test]
    fn demo_takes_the_r1_slugs_and_the_dotnet_flags() {
        let launch = parse(&["--demo", "late", "--dark", "--open-flyout"]).unwrap();
        assert_eq!(
            launch,
            Launch {
                demo: Some(DemoScenario::Late),
                data_dir: None,
                open_flyout: true,
                theme: ThemeMode::Dark,
                language: None,
                onboarding: false,
                demo_update: false,
                windows_startup: false,
                restarted_from: None,
                origin: Origin::Command,
            }
        );
        let demo = |arguments: &[&str]| parse(arguments).unwrap().demo.unwrap();
        assert_eq!(demo(&["--demo", "--dark"]), DemoScenario::OnTrack);
        assert_eq!(demo(&["--demo", "goal-reached"]), DemoScenario::Complete);
        assert_eq!(
            demo(&["--demo", "source-unavailable"]),
            DemoScenario::Unavailable
        );
        assert_eq!(demo(&["--demo-history-week"]), DemoScenario::HistoryWeek);
        assert_eq!(
            demo(&["--demo-problem-periods"]),
            DemoScenario::ProblemPeriods
        );
        assert_eq!(
            demo(&["--demo-fragmented-day"]),
            DemoScenario::FragmentedDay
        );
    }

    #[test]
    fn demo_update_asks_the_demo_for_an_update() {
        assert!(!parse(&["--demo"]).unwrap().demo_update);
        assert!(
            parse(&["--demo", "late", "--demo-update"])
                .unwrap()
                .demo_update
        );
    }

    #[test]
    fn onboarding_asks_for_the_onboarding() {
        assert!(!parse(&[]).unwrap().onboarding);
        assert!(parse(&["--onboarding", "--demo"]).unwrap().onboarding);
    }

    #[test]
    fn the_toast_activator_arguments_mark_a_start_by_com() {
        assert_eq!(parse(&[]).unwrap().origin, Origin::Command);
        let origin = |arguments: &[&str]| parse(arguments).unwrap().origin;
        assert_eq!(
            origin(&["--toast-activated", "-Embedding"]),
            Origin::ToastActivator
        );
        assert_eq!(origin(&["--toast-activated"]), Origin::ToastActivator);
        assert_eq!(origin(&["-Embedding"]), Origin::ToastActivator);
        let launch = parse(&[
            "--data-dir",
            "C:/Temp/wt",
            "--toast-activated",
            "-Embedding",
        ])
        .unwrap();
        assert_eq!(launch.data_dir, Some(PathBuf::from("C:/Temp/wt")));
        assert_eq!(launch.origin, Origin::ToastActivator);
    }

    #[test]
    fn only_the_first_click_of_a_process_com_started_is_cold() {
        let arrivals = |origin| {
            let activations = Activations::of(origin);
            [(); 3].map(|()| activations.arrived())
        };
        assert_eq!(
            arrivals(Origin::ToastActivator).map(Activation::name),
            ["cold", "warm", "warm"]
        );
        assert_eq!(
            arrivals(Origin::Command).map(Activation::name),
            ["warm", "warm", "warm"]
        );
    }

    #[test]
    fn refuses_unknown_arguments() {
        assert_eq!(
            parse(&["--sepia"]).unwrap_err().to_string(),
            "unknown argument \"--sepia\""
        );
        assert_eq!(
            parse(&["--demo", "sunny"]).unwrap_err().to_string(),
            "unknown demo scenario \"sunny\""
        );
    }
}
