//! The WinRT calls of the .NET `PackagedStartupTask` and `WindowsLaunchIntent.ResolveCurrentProcess`.
//! They need package identity; without it they fail, which reads as unavailable.

use std::io;

use windows::ApplicationModel::Activation::{
    ActivationKind, IActivatedEventArgs, StartupTaskActivatedEventArgs,
};
use windows::ApplicationModel::{AppInstance, StartupTask, StartupTaskState as WinRtState};
use windows::core::{HSTRING, Interface};

use super::{Activation, PackagedStartupTask, StartupTaskState};

/// A startup task declared in the package manifest, looked up again for every call as the .NET
/// port does, so a change made in Windows Settings meanwhile is seen.
pub struct WindowsStartupTask {
    task_id: HSTRING,
}

impl WindowsStartupTask {
    pub fn new(task_id: &str) -> Self {
        Self {
            task_id: HSTRING::from(task_id),
        }
    }

    fn task(&self) -> io::Result<StartupTask> {
        Ok(StartupTask::GetAsync(&self.task_id)?.join()?)
    }
}

impl PackagedStartupTask for WindowsStartupTask {
    fn state(&self) -> io::Result<StartupTaskState> {
        Ok(state(self.task()?.State()?))
    }

    fn request_enable(&self) -> io::Result<StartupTaskState> {
        Ok(state(self.task()?.RequestEnableAsync()?.join()?))
    }

    fn disable(&self) -> io::Result<StartupTaskState> {
        let task = self.task()?;
        task.Disable()?;
        Ok(state(task.State()?))
    }
}

fn state(state: WinRtState) -> StartupTaskState {
    match state {
        WinRtState::Disabled => StartupTaskState::Disabled,
        WinRtState::DisabledByUser => StartupTaskState::DisabledByUser,
        WinRtState::DisabledByPolicy => StartupTaskState::DisabledByPolicy,
        WinRtState::Enabled => StartupTaskState::Enabled,
        WinRtState::EnabledByPolicy => StartupTaskState::EnabledByPolicy,
        _ => StartupTaskState::Unrecognized,
    }
}

/// What Windows activated this process for. The portable build is only ever launched. A packaged
/// activation that cannot be read is [`Activation::Other`], which shows the window, as the .NET
/// app falls back to a manual launch.
pub fn current_activation() -> Activation {
    if !crate::has_package_identity() {
        return Activation::Launch;
    }
    AppInstance::GetActivatedEventArgs()
        .map_or(Activation::Other, |arguments| activation(&arguments))
}

fn activation(arguments: &IActivatedEventArgs) -> Activation {
    match arguments.Kind() {
        Ok(ActivationKind::Launch) => Activation::Launch,
        Ok(ActivationKind::StartupTask) => arguments
            .cast::<StartupTaskActivatedEventArgs>()
            .and_then(|startup| startup.TaskId())
            .map_or(Activation::Other, |task_id| {
                Activation::StartupTask(task_id.to_string_lossy())
            }),
        _ => Activation::Other,
    }
}

#[cfg(test)]
mod tests {
    //! The real WinRT calls in a test process, which has no package identity.

    use super::*;
    use crate::startup::{PACKAGED_TASK_ID, PackagedStartupRegistration, StartupRegistrationState};

    #[test]
    fn a_process_without_package_identity_was_launched() {
        assert!(!crate::has_package_identity());
        assert_eq!(current_activation(), Activation::Launch);
    }

    #[test]
    fn the_startup_task_is_unavailable_without_package_identity() {
        let registration =
            PackagedStartupRegistration::new(WindowsStartupTask::new(PACKAGED_TASK_ID));

        assert_eq!(registration.state(), StartupRegistrationState::Unavailable);
        assert_eq!(
            registration.set_enabled(true),
            StartupRegistrationState::Unavailable
        );
    }
}
