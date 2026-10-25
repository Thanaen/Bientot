use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

#[derive(Default)]
struct FakeRunKey {
    run_value: RefCell<Option<String>>,
    approval: Cell<Option<StartupApproval>>,
    set_count: Cell<usize>,
    delete_count: Cell<usize>,
}

impl FakeRunKey {
    fn with(run_value: &str, approval: StartupApproval) -> Self {
        let key = Self::default();
        key.run_value.replace(Some(run_value.to_owned()));
        key.approval.set(Some(approval));
        key
    }

    fn run_value(&self) -> Option<String> {
        self.run_value.borrow().clone()
    }
}

impl RunKey for &FakeRunKey {
    fn read_run_value(&self, _name: &str) -> io::Result<Option<String>> {
        Ok(self.run_value())
    }

    fn set_run_value(&self, _name: &str, command: &str) -> io::Result<()> {
        self.run_value.replace(Some(command.to_owned()));
        self.set_count.set(self.set_count.get() + 1);
        Ok(())
    }

    fn delete_run_value(&self, _name: &str) -> io::Result<()> {
        self.run_value.replace(None);
        self.delete_count.set(self.delete_count.get() + 1);
        Ok(())
    }

    fn read_approval(&self, _name: &str) -> io::Result<StartupApproval> {
        Ok(self.approval.get().unwrap_or(StartupApproval::None))
    }
}

/// A registry that cannot be reached, as when access is denied.
struct DeniedRunKey;

impl RunKey for DeniedRunKey {
    fn read_run_value(&self, _name: &str) -> io::Result<Option<String>> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn set_run_value(&self, _name: &str, _command: &str) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn delete_run_value(&self, _name: &str) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn read_approval(&self, _name: &str) -> io::Result<StartupApproval> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

/// A temporary folder with a stand-in executable whose path has a space.
struct Executable {
    directory: TempDir,
    path: PathBuf,
}

impl Executable {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("WorkTime Test.exe");
        fs::write(&path, [0x4d, 0x5a]).unwrap();
        Self { directory, path }
    }

    fn legacy(&self) -> String {
        format!("\"{}\"", self.path.display())
    }

    fn canonical(&self) -> String {
        format!("\"{}\" --worktime-windows-startup", self.path.display())
    }

    fn registration<'a>(&self, key: &'a FakeRunKey) -> PortableStartupRegistration<&'a FakeRunKey> {
        PortableStartupRegistration::new(key, "WorkTime.Test", Some(&self.path))
    }
}

fn approval_value(state: u32) -> Option<ApprovalValue> {
    let mut value = vec![0; 12];
    value[..4].copy_from_slice(&state.to_le_bytes());
    Some(ApprovalValue::Binary(value))
}

#[test]
fn the_command_quotes_an_absolute_executable_path_with_spaces() {
    let executable = Executable::new();

    let commands = StartupCommands::new(&executable.path).unwrap();

    assert_eq!(commands.canonical, executable.canonical());
    assert_eq!(commands.legacy, executable.legacy());
}

#[test]
fn the_command_rejects_development_hosts_and_non_executables() {
    let executable = Executable::new();
    for name in [
        "dotnet.exe",
        "testhost.exe",
        "vstest.console.exe",
        "MSBuild.EXE",
        "not-an-executable.dll",
    ] {
        let path = executable.directory.path().join(name);
        fs::write(&path, [0x4d, 0x5a]).unwrap();

        assert_eq!(StartupCommands::new(&path), None, "{name}");
    }
}

#[test]
fn the_command_rejects_a_relative_path() {
    assert_eq!(StartupCommands::new(Path::new("WorkTime.exe")), None);
}

#[test]
fn enable_and_disable_converge_without_rewriting_matching_state() {
    let executable = Executable::new();
    let key = FakeRunKey::default();
    let registration = executable.registration(&key);

    assert_eq!(registration.state(), StartupRegistrationState::Disabled);
    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Enabled
    );
    assert_eq!(key.run_value(), Some(executable.canonical()));
    assert_eq!(key.set_count.get(), 1);

    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Enabled
    );
    assert_eq!(key.set_count.get(), 1);

    assert_eq!(
        registration.set_enabled(false),
        StartupRegistrationState::Disabled
    );
    assert_eq!(key.delete_count.get(), 1);
    assert_eq!(
        registration.set_enabled(false),
        StartupRegistrationState::Disabled
    );
    assert_eq!(key.delete_count.get(), 1);
}

#[test]
fn enabling_does_not_rewrite_the_same_command_in_another_case() {
    let executable = Executable::new();
    let key = FakeRunKey::with(
        &executable.canonical().to_uppercase(),
        StartupApproval::None,
    );

    assert_eq!(
        executable.registration(&key).set_enabled(true),
        StartupRegistrationState::Enabled
    );
    assert_eq!(key.set_count.get(), 0);
}

#[test]
fn a_read_reports_a_mismatch_without_changing_it_and_an_explicit_enable_repairs_it() {
    let executable = Executable::new();
    let old = r#""C:\Old WorkTime\WorkTime.App.exe""#;
    let key = FakeRunKey::with(old, StartupApproval::None);
    let registration = executable.registration(&key);

    assert_eq!(
        registration.state(),
        StartupRegistrationState::RegistrationMismatch
    );
    assert_eq!(key.run_value().as_deref(), Some(old));
    assert_eq!(key.set_count.get(), 0);

    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Enabled
    );
    assert_eq!(key.run_value(), Some(executable.canonical()));
    assert_eq!(key.set_count.get(), 1);
}

#[test]
fn migration_rewrites_only_the_exact_owned_legacy_command() {
    let executable = Executable::new();
    for approval in [StartupApproval::None, StartupApproval::Enabled] {
        let key = FakeRunKey::with(&executable.legacy(), approval);
        let registration = executable.registration(&key);

        registration.migrate_owned_legacy_registration();
        registration.migrate_owned_legacy_registration();

        assert_eq!(
            key.run_value(),
            Some(executable.canonical()),
            "{approval:?}"
        );
        assert_eq!(key.set_count.get(), 1, "{approval:?}");
        assert_eq!(key.delete_count.get(), 0, "{approval:?}");
    }
}

#[test]
fn migration_preserves_the_legacy_command_when_the_windows_approval_does_not_allow_a_rewrite() {
    let executable = Executable::new();
    for approval in [StartupApproval::DisabledByUser, StartupApproval::Unknown] {
        let key = FakeRunKey::with(&executable.legacy(), approval);

        executable
            .registration(&key)
            .migrate_owned_legacy_registration();

        assert_eq!(key.run_value(), Some(executable.legacy()), "{approval:?}");
        assert_eq!(key.set_count.get(), 0, "{approval:?}");
        assert_eq!(key.delete_count.get(), 0, "{approval:?}");
    }
}

#[test]
fn migration_does_not_claim_another_executables_command() {
    let executable = Executable::new();
    let other = r#""C:\Other\Other.exe""#;
    let key = FakeRunKey::with(other, StartupApproval::Enabled);

    executable
        .registration(&key)
        .migrate_owned_legacy_registration();

    assert_eq!(key.run_value().as_deref(), Some(other));
    assert_eq!(key.set_count.get(), 0);
}

#[test]
fn disabling_does_not_remove_a_registration_owned_by_another_executable() {
    let executable = Executable::new();
    let other = r#""C:\Other\Other.exe""#;
    let key = FakeRunKey::with(other, StartupApproval::None);

    assert_eq!(
        executable.registration(&key).set_enabled(false),
        StartupRegistrationState::RegistrationMismatch
    );
    assert_eq!(key.run_value().as_deref(), Some(other));
    assert_eq!(key.delete_count.get(), 0);
}

#[test]
fn an_explicit_disable_removes_the_exact_owned_legacy_registration() {
    let executable = Executable::new();
    let key = FakeRunKey::with(&executable.legacy(), StartupApproval::None);

    assert_eq!(
        executable.registration(&key).set_enabled(false),
        StartupRegistrationState::Disabled
    );
    assert_eq!(key.run_value(), None);
    assert_eq!(key.delete_count.get(), 1);
}

#[test]
fn a_read_reports_startup_approval_overrides() {
    let executable = Executable::new();
    for (approval, expected) in [
        (
            StartupApproval::DisabledByUser,
            StartupRegistrationState::DisabledByUser,
        ),
        (
            StartupApproval::Unknown,
            StartupRegistrationState::Unavailable,
        ),
    ] {
        let key = FakeRunKey::with(&executable.canonical(), approval);

        assert_eq!(
            executable.registration(&key).state(),
            expected,
            "{approval:?}"
        );
    }
}

#[test]
fn a_disabled_legacy_registration_reports_the_windows_override_without_writing() {
    let executable = Executable::new();
    let key = FakeRunKey::with(&executable.legacy(), StartupApproval::DisabledByUser);

    assert_eq!(
        executable.registration(&key).state(),
        StartupRegistrationState::DisabledByUser
    );
    assert_eq!(key.run_value(), Some(executable.legacy()));
    assert_eq!(key.set_count.get(), 0);
    assert_eq!(key.delete_count.get(), 0);
}

#[test]
fn the_approval_decoder_recognizes_the_windows_states() {
    assert_eq!(
        decode_approval(approval_value(0x02)),
        StartupApproval::Enabled
    );
    assert_eq!(
        decode_approval(approval_value(0x06)),
        StartupApproval::Enabled
    );
    assert_eq!(
        decode_approval(approval_value(0x03)),
        StartupApproval::DisabledByUser
    );
}

#[test]
fn the_approval_decoder_treats_malformed_and_unknown_values_conservatively() {
    assert_eq!(decode_approval(None), StartupApproval::None);
    assert_eq!(
        decode_approval(Some(ApprovalValue::OtherType)),
        StartupApproval::Unknown
    );
    assert_eq!(
        decode_approval(Some(ApprovalValue::Binary(vec![0x06]))),
        StartupApproval::Unknown
    );
    assert_eq!(
        decode_approval(approval_value(0x07)),
        StartupApproval::Unknown
    );
}

#[test]
fn a_missing_or_hosted_executable_is_unavailable_and_never_writes() {
    let executable = Executable::new();
    let key = FakeRunKey::default();
    let missing = executable.directory.path().join("Missing.exe");
    for registration in [
        PortableStartupRegistration::new(&key, "WorkTime.Test", Some(&missing)),
        PortableStartupRegistration::new(&key, "WorkTime.Test", None),
    ] {
        assert_eq!(registration.state(), StartupRegistrationState::Unavailable);
        assert_eq!(
            registration.set_enabled(true),
            StartupRegistrationState::Unavailable
        );
    }
    assert_eq!(key.set_count.get(), 0);
}

#[test]
fn an_unreachable_registry_reads_as_unavailable() {
    let executable = Executable::new();
    let registration =
        PortableStartupRegistration::new(DeniedRunKey, "WorkTime.Test", Some(&executable.path));

    assert_eq!(registration.state(), StartupRegistrationState::Unavailable);
    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Unavailable
    );
    registration.migrate_owned_legacy_registration();
}

#[test]
fn the_start_with_windows_marker_selects_the_hidden_startup_for_an_ordinary_launch() {
    assert_eq!(
        LaunchIntent::resolve(true, &Activation::Launch),
        LaunchIntent::WindowsStartup
    );
}

#[test]
fn launches_that_windows_did_not_start_remain_visible() {
    for activation in [
        Activation::Launch,
        Activation::Other,
        Activation::StartupTask("AnotherStartupTask".to_owned()),
    ] {
        assert_eq!(
            LaunchIntent::resolve(false, &activation),
            LaunchIntent::Manual,
            "{activation:?}"
        );
    }
}

#[test]
fn the_exact_packaged_task_selects_the_hidden_startup() {
    assert_eq!(
        LaunchIntent::resolve(
            false,
            &Activation::StartupTask("WorkTimeStartupTask".to_owned())
        ),
        LaunchIntent::WindowsStartup
    );
}

#[test]
fn a_notification_activation_remains_visible_when_the_marker_is_present() {
    assert_eq!(
        LaunchIntent::resolve(true, &Activation::Other),
        LaunchIntent::Manual
    );
}

/// A startup task that Windows enables to `enabled_state` when asked.
struct FakeStartupTask {
    state: Cell<StartupTaskState>,
    enabled_state: StartupTaskState,
    enable_count: Cell<usize>,
    disable_count: Cell<usize>,
}

impl FakeStartupTask {
    fn new(state: StartupTaskState, enabled_state: StartupTaskState) -> Self {
        Self {
            state: Cell::new(state),
            enabled_state,
            enable_count: Cell::new(0),
            disable_count: Cell::new(0),
        }
    }
}

impl PackagedStartupTask for &FakeStartupTask {
    fn state(&self) -> io::Result<StartupTaskState> {
        Ok(self.state.get())
    }

    fn request_enable(&self) -> io::Result<StartupTaskState> {
        self.enable_count.set(self.enable_count.get() + 1);
        self.state.set(self.enabled_state);
        Ok(self.state.get())
    }

    fn disable(&self) -> io::Result<StartupTaskState> {
        self.disable_count.set(self.disable_count.get() + 1);
        self.state.set(StartupTaskState::Disabled);
        Ok(self.state.get())
    }
}

/// A startup task missing from the manifest, or a process without package identity.
struct MissingStartupTask;

impl PackagedStartupTask for MissingStartupTask {
    fn state(&self) -> io::Result<StartupTaskState> {
        Err(io::ErrorKind::NotFound.into())
    }

    fn request_enable(&self) -> io::Result<StartupTaskState> {
        Err(io::ErrorKind::NotFound.into())
    }

    fn disable(&self) -> io::Result<StartupTaskState> {
        Err(io::ErrorKind::NotFound.into())
    }
}

#[test]
fn every_startup_task_state_maps_to_a_registration_state() {
    for (task, expected) in [
        (
            StartupTaskState::Disabled,
            StartupRegistrationState::Disabled,
        ),
        (
            StartupTaskState::DisabledByUser,
            StartupRegistrationState::DisabledByUser,
        ),
        (
            StartupTaskState::DisabledByPolicy,
            StartupRegistrationState::DisabledByPolicy,
        ),
        (StartupTaskState::Enabled, StartupRegistrationState::Enabled),
        (
            StartupTaskState::EnabledByPolicy,
            StartupRegistrationState::EnabledByPolicy,
        ),
        (
            StartupTaskState::Unrecognized,
            StartupRegistrationState::Unavailable,
        ),
    ] {
        let fake = FakeStartupTask::new(task, StartupTaskState::Enabled);
        assert_eq!(
            PackagedStartupRegistration::new(&fake).state(),
            expected,
            "{task:?}"
        );
    }
}

#[test]
fn enabling_asks_windows_only_for_an_ordinary_disabled_task() {
    let task = FakeStartupTask::new(StartupTaskState::Disabled, StartupTaskState::Enabled);
    let registration = PackagedStartupRegistration::new(&task);

    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Enabled
    );
    assert_eq!(task.enable_count.get(), 1);
    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Enabled
    );
    assert_eq!(task.enable_count.get(), 1);
}

#[test]
fn enabling_keeps_the_denial_windows_returns() {
    let task = FakeStartupTask::new(StartupTaskState::Disabled, StartupTaskState::DisabledByUser);

    assert_eq!(
        PackagedStartupRegistration::new(&task).set_enabled(true),
        StartupRegistrationState::DisabledByUser
    );
    assert_eq!(task.enable_count.get(), 1);
}

#[test]
fn policy_and_user_controlled_states_are_not_overridden() {
    for (state, expected) in [
        (
            StartupTaskState::DisabledByUser,
            StartupRegistrationState::DisabledByUser,
        ),
        (
            StartupTaskState::DisabledByPolicy,
            StartupRegistrationState::DisabledByPolicy,
        ),
        (
            StartupTaskState::EnabledByPolicy,
            StartupRegistrationState::EnabledByPolicy,
        ),
    ] {
        let task = FakeStartupTask::new(state, StartupTaskState::Enabled);
        let registration = PackagedStartupRegistration::new(&task);

        assert_eq!(registration.set_enabled(true), expected, "{state:?}");
        assert_eq!(registration.set_enabled(false), expected, "{state:?}");
        assert_eq!(task.enable_count.get(), 0);
        assert_eq!(task.disable_count.get(), 0);
    }
}

#[test]
fn disabling_converges() {
    let task = FakeStartupTask::new(StartupTaskState::Enabled, StartupTaskState::Enabled);
    let registration = PackagedStartupRegistration::new(&task);

    assert_eq!(
        registration.set_enabled(false),
        StartupRegistrationState::Disabled
    );
    assert_eq!(task.disable_count.get(), 1);
    assert_eq!(
        registration.set_enabled(false),
        StartupRegistrationState::Disabled
    );
    assert_eq!(task.disable_count.get(), 1);
}

#[test]
fn an_unreachable_startup_task_reads_as_unavailable() {
    let registration = PackagedStartupRegistration::new(MissingStartupTask);

    assert_eq!(registration.state(), StartupRegistrationState::Unavailable);
    assert_eq!(
        registration.set_enabled(true),
        StartupRegistrationState::Unavailable
    );
}

#[cfg(windows)]
mod registry {
    //! The real `HKEY_CURRENT_USER` keys. They change the signed-in user's registry, so they run
    //! only with `BIENTOT_NATIVE_STARTUP_TEST=1`, like the .NET `NativeStartupFact` tests.

    use super::*;

    const APPROVED_PATH: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn enabled() -> bool {
        std::env::var("BIENTOT_NATIVE_STARTUP_TEST").as_deref() == Ok("1")
    }

    fn name() -> String {
        format!(
            "WorkTime.Tests.{:032x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    fn reg(arguments: &[&str]) {
        let status = std::process::Command::new("reg")
            .args(arguments)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "reg {arguments:?}");
    }

    fn set_approval(name: &str, state: u32) {
        let data = format!("{state:02x}{}", "0".repeat(22));
        reg(&[
            "add",
            &format!(r"HKCU\{APPROVED_PATH}"),
            "/v",
            name,
            "/t",
            "REG_BINARY",
            "/d",
            &data,
            "/f",
        ]);
    }

    fn delete_values(name: &str) {
        let _ = RegistryRunKey.delete_run_value(name);
        let _ = std::process::Command::new("reg")
            .args([
                "delete",
                &format!(r"HKCU\{APPROVED_PATH}"),
                "/v",
                name,
                "/f",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[test]
    fn the_owned_legacy_migration_preserves_the_windows_approval_and_disabled_state() {
        if !enabled() {
            return;
        }
        let executable = Executable::new();
        let name = name();
        let registration =
            PortableStartupRegistration::new(RegistryRunKey, name.clone(), Some(&executable.path));

        RegistryRunKey
            .set_run_value(&name, &executable.legacy())
            .unwrap();
        set_approval(&name, 0x06);
        registration.migrate_owned_legacy_registration();
        let migrated = RegistryRunKey.read_run_value(&name).unwrap();
        let enabled_approval = RegistryRunKey.read_approval(&name).unwrap();

        RegistryRunKey
            .set_run_value(&name, &executable.legacy())
            .unwrap();
        set_approval(&name, 0x03);
        registration.migrate_owned_legacy_registration();
        let preserved = RegistryRunKey.read_run_value(&name).unwrap();
        let disabled_approval = RegistryRunKey.read_approval(&name).unwrap();
        delete_values(&name);

        assert_eq!(migrated, Some(executable.canonical()));
        assert_eq!(enabled_approval, StartupApproval::Enabled);
        assert_eq!(preserved, Some(executable.legacy()));
        assert_eq!(disabled_approval, StartupApproval::DisabledByUser);
    }

    #[test]
    fn its_own_registry_entry_enables_disables_and_preserves_the_startup_approval() {
        if !enabled() {
            return;
        }
        let executable = Executable::new();
        let name = name();
        let registration =
            PortableStartupRegistration::new(RegistryRunKey, name.clone(), Some(&executable.path));

        let states = [
            registration.state(),
            registration.set_enabled(true),
            registration.set_enabled(true),
            {
                set_approval(&name, 0x06);
                registration.state()
            },
            {
                set_approval(&name, 0x03);
                registration.state()
            },
            registration.set_enabled(true),
            registration.set_enabled(false),
        ];
        let approval_after_disable = RegistryRunKey.read_approval(&name).unwrap();
        delete_values(&name);

        assert_eq!(
            states,
            [
                StartupRegistrationState::Disabled,
                StartupRegistrationState::Enabled,
                StartupRegistrationState::Enabled,
                StartupRegistrationState::Enabled,
                StartupRegistrationState::DisabledByUser,
                StartupRegistrationState::DisabledByUser,
                StartupRegistrationState::Disabled,
            ]
        );
        assert_eq!(approval_after_disable, StartupApproval::DisabledByUser);
    }
}
