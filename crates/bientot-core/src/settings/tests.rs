//! Ported from `SettingsWorkspaceTests.cs` of the first .NET app.

use std::collections::VecDeque;
use std::io;

use bientot_domain::configuration::{NotificationRule, Settings, SettingsError};
use bientot_store::settings::SaveSettingsError;
use jiff::{SignedDuration, Timestamp};

use super::number::{
    InputError, format_optional_target_duration, format_target_duration, parse_minutes,
    parse_optional_target_duration, parse_seconds, parse_target_duration,
};
use super::*;

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

/// Records the saves; fails them while `fail`. A refresh answers the next of `refreshes`, or
/// succeeds.
#[derive(Default)]
struct Port {
    saved: Vec<Settings>,
    attempts: usize,
    fail: bool,
    refreshes: VecDeque<bool>,
    refresh_count: usize,
}

impl Port {
    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    fn refreshing(refreshes: impl IntoIterator<Item = bool>) -> Self {
        Self {
            refreshes: refreshes.into_iter().collect(),
            ..Self::default()
        }
    }
}

impl SettingsPort for Port {
    fn save(&mut self, settings: &Settings) -> Result<(), SaveSettingsError> {
        self.attempts += 1;
        if self.fail {
            return Err(SaveSettingsError::Io(io::Error::other("disk unavailable")));
        }
        self.saved.push(settings.clone());
        Ok(())
    }

    fn refresh(&mut self) -> bool {
        self.refresh_count += 1;
        self.refreshes.pop_front().unwrap_or(true)
    }
}

fn save(workspace: &mut SettingsWorkspace, edit: SettingsEdit, port: &mut Port) {
    if let Some(pending) = workspace.submit(edit) {
        workspace.run(pending, port);
    }
}

fn status(workspace: &SettingsWorkspace, group: EditGroup) -> EditStatus {
    workspace.state(group).status
}

#[test]
fn a_valid_target_pair_saves_together() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::default();

    save(
        &mut workspace,
        SettingsEdit::Targets {
            target: parse_target_duration("7:24").unwrap(),
            minimum: parse_target_duration("6:12").unwrap(),
        },
        &mut port,
    );

    assert_eq!(port.saved.len(), 1);
    assert_eq!(port.saved[0].nominal_daily_target, minutes(444));
    assert_eq!(port.saved[0].minimum_effective_target, minutes(372));
    assert_eq!(status(&workspace, EditGroup::Targets), EditStatus::Saved);
    assert_eq!(workspace.summary(), Summary::Saved);
}

#[test]
fn invalid_targets_do_not_block_an_independent_notification_change() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::default();

    save(
        &mut workspace,
        SettingsEdit::Targets {
            target: minutes(420),
            minimum: minutes(480),
        },
        &mut port,
    );
    save(&mut workspace, SettingsEdit::DayComplete(true), &mut port);

    assert_eq!(
        workspace.state(EditGroup::Targets),
        GroupState {
            generation: 1,
            status: EditStatus::Invalid,
            error: Some(EditError::Settings(SettingsError::Minimum)),
        }
    );
    assert_eq!(
        status(&workspace, EditGroup::DayComplete),
        EditStatus::Saved
    );
    assert!(workspace.current().notifications.day_complete.enabled);
    assert_eq!(workspace.current().minimum_effective_target, minutes(360));
    assert_eq!(workspace.summary(), Summary::Unsettled);
}

#[test]
fn an_older_completion_does_not_clear_a_newer_failed_draft() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let first = workspace.submit(SettingsEdit::Margin(minutes(20))).unwrap();
    let second = workspace.submit(SettingsEdit::Margin(minutes(25))).unwrap();

    workspace.run(first, &mut Port::default());
    assert_eq!(status(&workspace, EditGroup::Margin), EditStatus::Saving);
    workspace.run(second, &mut Port::failing());

    assert_eq!(
        status(&workspace, EditGroup::Margin),
        EditStatus::SaveFailed
    );
    assert_eq!(workspace.current().orange_warning_margin, minutes(20));

    let retry = workspace.retry(EditGroup::Margin).unwrap();
    workspace.run(retry, &mut Port::default());
    assert_eq!(status(&workspace, EditGroup::Margin), EditStatus::Saved);
    assert_eq!(workspace.current().orange_warning_margin, minutes(25));
}

#[test]
fn closing_waits_for_an_accepted_save_and_refuses_failed_or_invalid_drafts() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let save = workspace.submit(SettingsEdit::Credit(false)).unwrap();
    assert!(workspace.is_saving());
    assert_eq!(workspace.summary(), Summary::Saving);

    workspace.run(save, &mut Port::default());
    assert!(!workspace.is_saving());
    assert!(!workspace.has_unsaved_changes());

    workspace.submit(SettingsEdit::Targets {
        target: SignedDuration::ZERO,
        minimum: minutes(360),
    });
    assert!(workspace.has_unsaved_changes());
    workspace.discard_unsaved();
    assert!(!workspace.has_unsaved_changes());
    assert_eq!(workspace.summary(), Summary::Saved);
}

#[test]
fn a_new_invalid_draft_removes_the_older_failed_retry() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::failing();
    save(&mut workspace, SettingsEdit::Margin(minutes(20)), &mut port);

    workspace.mark_invalid(
        EditGroup::Margin,
        EditError::Input(InputError::WholeNumberRequired),
    );
    port.fail = false;

    assert_eq!(workspace.retry(EditGroup::Margin), None);
    assert_eq!(status(&workspace, EditGroup::Margin), EditStatus::Invalid);
    assert_eq!(workspace.current().orange_warning_margin, minutes(15));
    assert_eq!(port.attempts, 1);
}

#[test]
fn a_pending_refresh_is_retried_without_writing_the_settings_again() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::refreshing([false]);

    save(&mut workspace, SettingsEdit::Credit(false), &mut port);
    assert_eq!(
        status(&workspace, EditGroup::Credit),
        EditStatus::RefreshPending
    );
    assert_eq!(workspace.summary(), Summary::Unsettled);

    save(&mut workspace, SettingsEdit::Credit(false), &mut port);
    assert_eq!(
        status(&workspace, EditGroup::Credit),
        EditStatus::RefreshPending
    );
    assert_eq!((port.saved.len(), port.refresh_count), (1, 1));

    let retry = workspace.retry(EditGroup::Credit).unwrap();
    workspace.run(retry, &mut port);
    assert_eq!(status(&workspace, EditGroup::Credit), EditStatus::Saved);
    assert_eq!((port.saved.len(), port.refresh_count), (1, 2));
}

#[test]
fn a_recalculated_edit_clears_another_groups_pending_refresh() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::refreshing([false, true]);

    save(&mut workspace, SettingsEdit::Margin(minutes(20)), &mut port);
    assert_eq!(
        status(&workspace, EditGroup::Margin),
        EditStatus::RefreshPending
    );
    save(&mut workspace, SettingsEdit::Credit(false), &mut port);

    assert_eq!(status(&workspace, EditGroup::Margin), EditStatus::Saved);
    assert_eq!(status(&workspace, EditGroup::Credit), EditStatus::Saved);
    assert_eq!((port.saved.len(), port.refresh_count), (2, 2));
}

#[test]
fn a_successful_retry_settles_pending_groups_without_overwriting_newer_states() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::refreshing([false, false]);
    save(&mut workspace, SettingsEdit::Margin(minutes(20)), &mut port);
    save(&mut workspace, SettingsEdit::Credit(false), &mut port);
    port.fail = true;
    save(&mut workspace, SettingsEdit::DayComplete(true), &mut port);
    port.fail = false;

    let retry = workspace.retry(EditGroup::Margin).unwrap();
    workspace.mark_invalid(
        EditGroup::Margin,
        EditError::Input(InputError::WholeNumberRequired),
    );
    let saving = workspace.submit(SettingsEdit::LongLock(minutes(75)));
    workspace.run(retry, &mut port);

    assert_eq!(status(&workspace, EditGroup::Margin), EditStatus::Invalid);
    assert_eq!(status(&workspace, EditGroup::Credit), EditStatus::Saved);
    assert_eq!(
        status(&workspace, EditGroup::DayComplete),
        EditStatus::SaveFailed
    );
    assert_eq!(status(&workspace, EditGroup::LongLock), EditStatus::Saving);
    assert!(saving.is_some());
}

#[test]
fn debt_settings_save_together_and_an_invalid_limit_is_refused() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::default();

    save(
        &mut workspace,
        SettingsEdit::Debt {
            enabled: true,
            limit: Some(minutes(25 * 60)),
        },
        &mut port,
    );
    assert_eq!(
        workspace.state(EditGroup::Debt).error,
        Some(EditError::Settings(SettingsError::Debt))
    );
    assert_eq!(
        workspace
            .state(EditGroup::Debt)
            .error
            .unwrap()
            .resource_key(),
        "SettingsDebt"
    );
    assert!(port.saved.is_empty());

    save(
        &mut workspace,
        SettingsEdit::Debt {
            enabled: true,
            limit: Some(minutes(120)),
        },
        &mut port,
    );

    assert_eq!(port.saved.len(), 1);
    assert!(port.saved[0].weekly_debt_enabled);
    assert_eq!(port.saved[0].maximum_weekly_debt, Some(minutes(120)));
    assert_eq!(status(&workspace, EditGroup::Debt), EditStatus::Saved);
}

#[test]
fn the_updates_toggle_saves_the_choice_and_keeps_the_last_check() {
    let last_checked: Timestamp = "2026-10-11T07:30:00Z".parse().unwrap();
    let mut settings = Settings::default();
    settings.updates.last_checked_at = Some(last_checked);
    let mut workspace = SettingsWorkspace::new(settings);
    let mut port = Port::default();

    save(&mut workspace, SettingsEdit::Updates(false), &mut port);

    assert!(!port.saved[0].updates.check_for_updates);
    assert_eq!(port.saved[0].updates.last_checked_at, Some(last_checked));
}

#[test]
fn the_usage_ping_and_the_activity_source_save_on_their_own() {
    let settings = Settings {
        activity_source: None,
        ..Settings::default()
    };
    let mut workspace = SettingsWorkspace::new(settings);
    let mut port = Port::default();

    save(&mut workspace, SettingsEdit::UsagePing(false), &mut port);
    save(
        &mut workspace,
        SettingsEdit::ActivitySource(ActivitySourceKind::BuiltIn),
        &mut port,
    );

    assert!(!port.saved[0].usage_ping);
    assert_eq!(port.saved[0].activity_source, None);
    assert!(!port.saved[1].usage_ping);
    assert_eq!(
        port.saved[1].activity_source,
        Some(ActivitySourceKind::BuiltIn)
    );
    assert_eq!(status(&workspace, EditGroup::UsagePing), EditStatus::Saved);
    assert_eq!(
        status(&workspace, EditGroup::ActivitySource),
        EditStatus::Saved
    );
}

#[test]
fn a_startup_change_saves_once_windows_applied_it_and_keeps_a_refusal_to_retry() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::default();

    let pending = workspace
        .submit(SettingsEdit::StartWithWindows(false))
        .unwrap();
    assert_eq!(pending.startup(), Some(false));
    workspace.finish_startup(pending, false, &mut port);

    assert!(port.saved.is_empty());
    assert_eq!(
        workspace.state(EditGroup::Startup),
        GroupState {
            generation: 1,
            status: EditStatus::SaveFailed,
            error: Some(EditError::StartupChangeFailed),
        }
    );
    assert_eq!(
        EditError::StartupChangeFailed.resource_key(),
        "SettingsStartupChangeFailed"
    );

    let retry = workspace.retry(EditGroup::Startup).unwrap();
    assert_eq!(retry.startup(), Some(false));
    workspace.finish_startup(retry, true, &mut port);

    assert_eq!(port.saved.len(), 1);
    assert!(!port.saved[0].start_with_windows);
    assert_eq!(status(&workspace, EditGroup::Startup), EditStatus::Saved);
    let other = workspace.submit(SettingsEdit::UsagePing(false)).unwrap();
    assert_eq!(other.startup(), None);
}

#[test]
fn a_failed_save_is_retried_without_touching_other_groups() {
    let mut workspace = SettingsWorkspace::new(Settings::default());
    let mut port = Port::failing();

    save(&mut workspace, SettingsEdit::Updates(false), &mut port);
    assert_eq!(
        workspace.state(EditGroup::Updates),
        GroupState {
            generation: 1,
            status: EditStatus::SaveFailed,
            error: Some(EditError::SaveFailed),
        }
    );
    assert_eq!(EditError::SaveFailed.resource_key(), "SettingsSaveFailed");
    assert_eq!(status(&workspace, EditGroup::Targets), EditStatus::Saved);
    assert!(workspace.current().updates.check_for_updates);
    assert!(workspace.has_unsaved_changes());
    port.fail = false;

    let retry = workspace.retry(EditGroup::Updates).unwrap();
    assert_eq!(status(&workspace, EditGroup::Updates), EditStatus::Saving);
    workspace.run(retry, &mut port);

    assert_eq!(status(&workspace, EditGroup::Updates), EditStatus::Saved);
    assert!(!workspace.current().updates.check_for_updates);
    assert_eq!(port.attempts, 2);
}

#[test]
fn edits_change_only_their_own_settings() {
    let settings = Settings::default();
    let rule = NotificationRule::new(false, minutes(30));

    let edited = SettingsEdit::DepartureApproach(rule).apply(&settings);
    assert_eq!(edited.notifications.latest_departure_approach, rule);
    assert_eq!(
        edited.notifications.completion_approach,
        settings.notifications.completion_approach
    );

    let edited = SettingsEdit::DayComplete(true).apply(&settings);
    assert_eq!(
        edited.notifications.day_complete,
        NotificationRule::new(true, SignedDuration::ZERO)
    );

    let edited = SettingsEdit::Language(Some("en-US".to_owned())).apply(&settings);
    assert_eq!(edited.language_override.as_deref(), Some("en-US"));
    assert_eq!(
        Settings {
            language_override: None,
            ..edited
        },
        settings
    );
}

#[test]
fn the_number_parser_tells_malformed_from_out_of_range_and_accepts_zero() {
    for (raw, expected) in [
        ("", Err(InputError::WholeNumberRequired)),
        ("5.5", Err(InputError::WholeNumberRequired)),
        ("1E300", Err(InputError::NumberOutOfRange)),
        ("-1", Err(InputError::NumberOutOfRange)),
        ("0", Ok(SignedDuration::ZERO)),
        ("15", Ok(minutes(15))),
        (" 15 ", Ok(minutes(15))),
    ] {
        assert_eq!(parse_minutes(raw, 0., 1440.), expected, "{raw:?}");
    }
    assert_eq!(
        parse_seconds("abc", 30., 1800.),
        Err(InputError::WholeSecondsRequired)
    );
    assert_eq!(
        parse_seconds("180", 30., 1800.),
        Ok(SignedDuration::from_secs(180))
    );
    assert_eq!(
        InputError::WholeNumberRequired.resource_key(),
        "SettingsWholeNumberRequired"
    );
}

#[test]
fn the_target_parser_accepts_its_boundaries() {
    for (raw, expected) in [
        ("0:01", 1),
        ("7:24", 444),
        ("07:24", 444),
        (" 7:24 ", 444),
        ("23:59", 1439),
        ("24:00", 1440),
    ] {
        assert_eq!(parse_target_duration(raw), Ok(minutes(expected)), "{raw:?}");
    }
}

#[test]
fn the_target_parser_refuses_malformed_and_out_of_range_drafts() {
    use InputError::{TargetDurationOutOfRange as OutOfRange, TargetDurationRequired as Required};
    for (raw, expected) in [
        ("", Required),
        ("7:2", Required),
        ("7:240", Required),
        ("7.24", Required),
        ("7:2.4", Required),
        ("000:01", Required),
        ("é:00", Required),
        ("0:00", OutOfRange),
        ("7:60", OutOfRange),
        ("24:01", OutOfRange),
        ("25:00", OutOfRange),
    ] {
        assert_eq!(parse_target_duration(raw), Err(expected), "{raw:?}");
    }
    assert_eq!(
        OutOfRange.resource_key(),
        "SettingsTargetDurationOutOfRange"
    );
}

#[test]
fn the_target_formatter_writes_hours_and_two_digits_of_minutes() {
    assert_eq!(format_target_duration(minutes(1)), "0:01");
    assert_eq!(format_target_duration(minutes(444)), "7:24");
    assert_eq!(format_target_duration(minutes(1440)), "24:00");
}

#[test]
fn the_debt_limit_parser_reads_blank_as_no_limit() {
    for (raw, expected) in [
        ("", Ok(None)),
        ("  ", Ok(None)),
        ("1:05", Ok(Some(minutes(65)))),
        ("0:00", Err(InputError::TargetDurationOutOfRange)),
        ("1h", Err(InputError::TargetDurationRequired)),
    ] {
        assert_eq!(parse_optional_target_duration(raw), expected, "{raw:?}");
    }
    assert_eq!(format_optional_target_duration(None), "");
    assert_eq!(format_optional_target_duration(Some(minutes(65))), "1:05");
}

#[test]
fn the_language_is_the_command_then_the_saved_one_then_windows_then_english() {
    use super::language::resolve;
    assert_eq!(resolve(true, None, ["fr-FR"]), "en-US");
    assert_eq!(resolve(false, Some("fr-FR"), ["en-US"]), "fr-FR");
    assert_eq!(resolve(false, None, ["de-DE", "fr-CA"]), "fr-FR");
    assert_eq!(resolve(false, None, ["FR"]), "fr-FR");
    assert_eq!(resolve(false, Some("de-DE"), ["de-DE"]), "en-US");
    assert_eq!(resolve(false, None, []), "en-US");
}
