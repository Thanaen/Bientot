//! What scenarios of separate files share: the launch on Saturday 10 October 2026 and the
//! states of « Paramètres », whose settings the onboarding opens on.

use anyhow::Context as _;
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use worktime_app::demo;
use worktime_app::live::{self, LiveHost};
use worktime_app::settings::{Field, Switch, UsagePingFacts};
use worktime_app::settings_view::SettingsView;
use worktime_app::theme::motion;
use worktime_core::settings::{EditGroup, SettingsPort};
use worktime_domain::configuration::{RecurringAppointment, WorkTimeSettings};
use worktime_domain::sources::{ActivitySourceKind, ActivitySourceSelection, IdleSpan, IdleState};
use worktime_domain::time::{Clock, FixedTimeZoneProvider, FrozenClock};
use worktime_domain::usage_ping::{Build, Channel};
use worktime_store::activity::{ActivitySourceSelectionRepository, ActivitySpanRepository};
use worktime_store::data_directory::DataDirectory;
use worktime_store::settings::SaveSettingsError;
use worktime_windows::startup::StartupRegistrationState;

use super::{DIALOG_FRAMES, MENU_FRAMES};

/// The week a launch on Saturday 10 October 2026 has behind it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Behind {
    Nothing,
    /// The built-in source from Monday 5 October 2026, active 9:00 to 12:30 and 13:30 to 17:00
    /// UTC from Monday to Friday and idle around, up to the clock.
    Worked,
    /// The worked week, and on Friday evening an hour from 21:00 after which nothing was
    /// recorded until midnight: a gap to settle that closes the day.
    WorkedToMidnight,
}

/// `then` on the host of a launch on Saturday 10 October 2026 at 10:00 UTC, on `directory`, with
/// the built-in source as the onboarding selects it and `behind` it, once it refreshed.
pub(super) fn on_saturday<R>(
    directory: &std::path::Path,
    behind: Behind,
    then: impl FnOnce(&mut LiveHost<'_>) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    let directory = DataDirectory::from(directory.to_owned());
    let mut loaded = live::load_settings(&directory)?;
    loaded.settings.activity_source = Some(ActivitySourceKind::BuiltIn);
    let connection = worktime_store::database::open(&directory.database_file())?;
    let clock = FrozenClock::new("2026-10-10T10:00:00Z".parse()?);
    worktime_store::database::migrate(&connection)?;
    seed_worked_week(&connection, &clock, behind)?;
    let zones = FixedTimeZoneProvider::new("UTC", TimeZone::UTC);
    let mut host = LiveHost::start(
        &directory,
        loaded,
        &connection,
        &clock,
        &zones,
        &demo::Typing,
    )?;
    // A first sample: without it, the seeded spans read as recording that stopped at 10:00.
    host.poll();
    host.refresh();
    let result = then(&mut host);
    host.shut_down();
    result
}

/// The activity `behind` describes, recorded up to the clock.
fn seed_worked_week(
    connection: &rusqlite::Connection,
    clock: &dyn Clock,
    behind: Behind,
) -> anyhow::Result<()> {
    if behind == Behind::Nothing {
        return Ok(());
    }
    let monday = jiff::civil::date(2026, 10, 5);
    let at = |day: i64, hour, minute| -> anyhow::Result<jiff::Timestamp> {
        Ok(monday
            .checked_add(jiff::Span::new().days(day))?
            .at(hour, minute, 0, 0)
            .to_zoned(TimeZone::UTC)?
            .timestamp())
    };
    ActivitySourceSelectionRepository::new(connection, clock).append(ActivitySourceSelection {
        effective_from: at(0, 0, 0)?,
        source: ActivitySourceKind::BuiltIn,
    })?;
    let mut spans = Vec::new();
    let mut idle_from = at(0, 0, 0)?;
    for day in 0..5 {
        spans.extend([
            (IdleState::Idle, idle_from, at(day, 9, 0)?),
            (IdleState::Active, at(day, 9, 0)?, at(day, 12, 30)?),
            (IdleState::Idle, at(day, 12, 30)?, at(day, 13, 30)?),
            (IdleState::Active, at(day, 13, 30)?, at(day, 17, 0)?),
        ]);
        idle_from = at(day, 17, 0)?;
    }
    if behind == Behind::WorkedToMidnight {
        spans.extend([
            (IdleState::Idle, idle_from, at(4, 21, 0)?),
            (IdleState::Active, at(4, 21, 0)?, at(4, 22, 0)?),
        ]);
        idle_from = at(5, 0, 0)?;
    }
    spans.push((IdleState::Idle, idle_from, clock.now()));
    let repository = ActivitySpanRepository::new(connection, clock);
    for (state, start, end) in spans {
        let span = IdleSpan::new(state, start, end).context("a seeded span is not empty")?;
        repository.append(span, None)?;
    }
    Ok(())
}

/// The states of the mockup's « Paramètres » board.
#[derive(Clone, Copy)]
pub(super) enum SettingsState {
    Saved,
    Saving,
    FieldError,
    SaveFailed,
    Unsaved,
    DepartureOff,
    /// The settings still name ActivityWatch.
    ActivityWatch,
    /// « Voir ce qui est envoyé » open, the page scrolled to its end.
    DetailsOpen,
    /// `DetailsOpen` with the signal off.
    UsageOff,
    /// `DetailsOpen` before the first signal.
    NeverSent,
    /// Start with Windows in this state; every other state shows it enabled.
    Startup(StartupRegistrationState),
    /// Windows left start with Windows disabled by the user when asked to enable it.
    StartupRefused,
    /// The ZIP build, which has no updates to ask for, scrolled to the end.
    Zip,
    /// `DetailsOpen` on the ZIP build.
    ZipDetailsOpen,
    /// `DetailsOpen` on the build Velopack's `Setup.exe` installed.
    DirectDetailsOpen,
    /// Scrolled to the end.
    End,
    /// `settings.json` with a line wider than its box.
    Wrapped,
    /// The language list open under its button.
    LanguageOpen,
    /// Two recurring appointments, the second switched off.
    Appointments,
    /// `Appointments` with the editor open on a new one.
    AppointmentNew,
    /// `Appointments` with the editor open on the first.
    AppointmentEdit,
    /// `AppointmentNew` saved with hours that do not follow each other.
    AppointmentHoursError,
    /// `AppointmentEdit` with « Supprimer » pressed once.
    AppointmentDeleteConfirm,
    /// `Appointments` with the editor open on the second, one Monday in two.
    AppointmentAlternate,
}

impl SettingsState {
    pub(super) fn slug(self) -> Option<&'static str> {
        match self {
            Self::Saved => None,
            Self::Saving => Some("saving"),
            Self::FieldError => Some("field-error"),
            Self::SaveFailed => Some("save-failed"),
            Self::Unsaved => Some("unsaved"),
            Self::DepartureOff => Some("departure-off"),
            Self::ActivityWatch => Some("activitywatch"),
            Self::DetailsOpen => Some("details-open"),
            Self::UsageOff => Some("usage-off"),
            Self::NeverSent => Some("never-sent"),
            Self::Startup(state) => Some(match state {
                StartupRegistrationState::DisabledByUser => "startup-disabled-by-user",
                StartupRegistrationState::DisabledByPolicy => "startup-disabled-by-policy",
                StartupRegistrationState::EnabledByPolicy => "startup-enabled-by-policy",
                StartupRegistrationState::RegistrationMismatch => "startup-mismatch",
                StartupRegistrationState::Unavailable => "startup-unavailable",
                StartupRegistrationState::Disabled => "startup-disabled",
                StartupRegistrationState::Enabled => "startup-enabled",
            }),
            Self::StartupRefused => Some("startup-change-failed"),
            Self::Zip => Some("zip"),
            Self::ZipDetailsOpen => Some("zip-details-open"),
            Self::DirectDetailsOpen => Some("direct-details-open"),
            Self::End => Some("end"),
            Self::Wrapped => Some("wrapped"),
            Self::LanguageOpen => Some("language-open"),
            Self::Appointments => Some("appointments"),
            Self::AppointmentNew => Some("appointment-new"),
            Self::AppointmentEdit => Some("appointment-edit"),
            Self::AppointmentHoursError => Some("appointment-hours-error"),
            Self::AppointmentDeleteConfirm => Some("appointment-delete-confirm"),
            Self::AppointmentAlternate => Some("appointment-alternate"),
        }
    }

    /// The entrance this state starts and the frames of it to capture, in percent.
    pub(super) fn entrance(self) -> Option<(motion::Timing, [u32; 5])> {
        match self {
            Self::Unsaved => Some((motion::DIALOG, DIALOG_FRAMES)),
            Self::LanguageOpen => Some((motion::MENU, MENU_FRAMES)),
            _ => None,
        }
    }

    /// The settings of the mockup, with the departure notification off for `DepartureOff`,
    /// ActivityWatch selected for `ActivityWatch` and the signal off for `UsageOff`.
    pub(super) fn settings(self) -> WorkTimeSettings {
        let mut settings = WorkTimeSettings {
            nominal_daily_target: jiff::SignedDuration::from_mins(7 * 60 + 24),
            minimum_effective_target: jiff::SignedDuration::from_mins(6 * 60 + 12),
            activity_source: Some(ActivitySourceKind::BuiltIn),
            ..WorkTimeSettings::default()
        };
        match self {
            Self::DepartureOff => settings.notifications.latest_departure_approach.enabled = false,
            Self::ActivityWatch => {
                settings.activity_source = Some(ActivitySourceKind::ActivityWatch)
            }
            Self::UsageOff => settings.usage_ping = false,
            Self::Appointments
            | Self::AppointmentNew
            | Self::AppointmentEdit
            | Self::AppointmentHoursError
            | Self::AppointmentDeleteConfirm
            | Self::AppointmentAlternate => {
                let appointment = |id: &str, name: &str, weekday, start: i8, end: i8, enabled| {
                    RecurringAppointment {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        weekday,
                        start: jiff::civil::time(start, 0, 0, 0),
                        end: jiff::civil::time(end, 0, 0, 0),
                        enabled,
                        since: Some(jiff::civil::date(2026, 10, 1)),
                        alternate_weeks_from: None,
                        automatic: false,
                    }
                };
                settings.recurring_appointments = vec![
                    RecurringAppointment {
                        automatic: true,
                        ..appointment(
                            "thursday-1400",
                            "Réunion d’équipe",
                            Weekday::Thursday,
                            14,
                            16,
                            true,
                        )
                    },
                    RecurringAppointment {
                        alternate_weeks_from: Some(jiff::civil::date(2026, 10, 5)),
                        ..appointment("monday-0930", "", Weekday::Monday, 9, 10, false)
                    },
                ];
            }
            _ => {}
        }
        settings
    }

    /// The signal of the mockup: version 1.0.0 from the Store (the ZIP for `Zip`), or
    /// 0.1.0-alpha.242 from the installer, published on 1 October, for `DirectDetailsOpen`; last
    /// sent on a fixed day, or never for `NeverSent`.
    pub(super) fn usage_ping(self) -> UsagePingFacts {
        let today = jiff::civil::date(2026, 10, 13);
        let channel = match self {
            Self::Zip | Self::ZipDetailsOpen => Channel::Zip,
            Self::DirectDetailsOpen => Channel::Direct,
            _ => Channel::Store,
        };
        // Only scripts/release-direct.sh passes the day of its commit.
        let (version, released) = match channel {
            Channel::Direct => ("0.1.0-alpha.242", Some(jiff::civil::date(2026, 10, 1))),
            Channel::Store | Channel::Zip => ("1.0.0", None),
        };
        UsagePingFacts {
            build: Build::new(version, channel).expect("the service accepts the version"),
            released,
            today,
            last_sent: (!matches!(self, Self::NeverSent)).then_some(today),
        }
    }

    pub(super) fn apply(
        self,
        view: &mut SettingsView,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<SettingsView>,
    ) {
        if matches!(self, Self::FieldError | Self::Unsaved) {
            view.type_text(Field::Target, "25:00", cx);
        }
        if let Self::SaveFailed = self {
            view.type_text(Field::Margin, "20", cx);
        }
        let details = matches!(
            self,
            Self::DetailsOpen
                | Self::UsageOff
                | Self::NeverSent
                | Self::ZipDetailsOpen
                | Self::DirectDetailsOpen
        );
        if details || matches!(self, Self::End | Self::Zip) {
            view.scroll_to_end();
        }
        match self {
            Self::AppointmentNew | Self::AppointmentHoursError => {
                view.open_appointment(None, window, cx);
            }
            Self::AppointmentEdit | Self::AppointmentDeleteConfirm => {
                view.open_appointment(Some("thursday-1400"), window, cx);
            }
            Self::AppointmentAlternate => {
                view.open_appointment(Some("monday-0930"), window, cx);
            }
            _ => {}
        }
        let form = view.form_mut();
        form.show_startup(match self {
            Self::Startup(state) => state,
            Self::StartupRefused => StartupRegistrationState::Disabled,
            _ => StartupRegistrationState::Enabled,
        });
        match self {
            Self::Saved
            | Self::DepartureOff
            | Self::ActivityWatch
            | Self::End
            | Self::Wrapped
            | Self::Zip
            | Self::Startup(_)
            | Self::Appointments
            | Self::AppointmentNew
            | Self::AppointmentEdit
            | Self::AppointmentAlternate => {}
            Self::AppointmentHoursError => {
                *form.time_mut(worktime_app::settings::Clock::AppointmentEnd) =
                    worktime_app::adjust::TimeField::new(13, 0);
                form.save_appointment("");
            }
            Self::AppointmentDeleteConfirm => {
                form.delete_appointment();
            }
            Self::StartupRefused => {
                let pending = form.toggle(Switch::Startup);
                let pending = pending.expect("a disabled registration can change");
                form.finish_startup(
                    pending,
                    StartupRegistrationState::DisabledByUser,
                    &mut FailingDisk,
                );
            }
            Self::DetailsOpen
            | Self::UsageOff
            | Self::NeverSent
            | Self::ZipDetailsOpen
            | Self::DirectDetailsOpen => form.toggle_details(),
            Self::Saving => {
                form.commit(EditGroup::Margin);
            }
            Self::FieldError => {
                form.commit(EditGroup::Targets);
            }
            Self::SaveFailed => {
                let pending = form.commit(EditGroup::Margin);
                form.run(pending.expect("the margin is valid"), &mut FailingDisk);
            }
            Self::LanguageOpen => view.toggle_language_list(cx),
            Self::Unsaved => {
                form.commit(EditGroup::Targets);
                view.should_close(window, cx);
            }
        }
    }
}

/// A disk that refuses every save.
struct FailingDisk;

impl SettingsPort for FailingDisk {
    fn save(&mut self, _: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        Err(SaveSettingsError::Io(std::io::Error::other(
            "disk unavailable",
        )))
    }

    fn refresh(&mut self) -> bool {
        true
    }
}
