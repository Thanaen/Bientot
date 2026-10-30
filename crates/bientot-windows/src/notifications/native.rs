//! The WinRT calls that stand in for the .NET `AppNotificationManager`.

mod activator;

use std::path::Path;
use std::sync::Arc;

use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{
    NotificationSetting, ToastNotification, ToastNotificationManager, ToastNotifier,
};
use windows::Win32::Foundation::E_FAIL;
use windows::core::{Error, GUID, HSTRING};

use self::activator::ActivatorRegistration;
use super::portable::RegistrationPlan;
use super::{NotificationRequest, PACKAGED_ACTIVATOR, toast_xml};

/// Who Windows shows the toasts as, and the COM class it hands their clicks to.
pub enum Sender {
    /// The MSIX build, whose manifest declares both.
    Package,
    /// The portable build, which registers both under `HKEY_CURRENT_USER`.
    Portable(RegistrationPlan),
}

/// The icon of the portable build's toasts, which its ZIP ships next to the executable.
const PORTABLE_ICON: &str = "worktime.png";

impl Sender {
    /// This process's package, or else the portable build at this executable's path, which a
    /// click starts on `data_dir` when given.
    pub fn current(data_dir: Option<&Path>) -> windows::core::Result<Self> {
        if crate::has_package_identity() {
            return Ok(Self::Package);
        }
        let exe = std::env::current_exe()?;
        let icon = exe.with_file_name(PORTABLE_ICON);
        Ok(Self::Portable(RegistrationPlan::portable(
            exe,
            icon,
            data_dir.map(Path::to_path_buf),
        )))
    }
}

/// Shows WorkTime's toasts and calls back when the user clicks one.
pub struct Toasts {
    notifier: ToastNotifier,
    /// Whether `Setting` can tell a refused toast: only with package identity. Without it,
    /// Windows answers `Setting` only once the AppUserModelID has shown a toast, so checking it
    /// first would refuse every toast forever.
    reads_setting: bool,
    /// Receives the clicks on the popup and on the toasts kept in Notification Center alike. The
    /// toast's own `Activated` event goes unheard: Windows raises it too for a toast the app still
    /// holds, so listening to it would report one click twice.
    #[expect(
        dead_code,
        reason = "held so that dropping the toasts revokes the class"
    )]
    activator: ActivatorRegistration,
}

impl Toasts {
    /// Registers with Windows as `sender`. `activated` receives the clicked toast's delivery id,
    /// on a COM thread rather than the UI thread.
    ///
    /// Call it before the process opens any window: Microsoft requires a process to set its
    /// AppUserModelID before it shows any UI, and when COM started the process for a click, the
    /// click waits for the class.
    pub fn register(
        sender: Sender,
        activated: impl Fn(Option<String>) + Send + Sync + 'static,
    ) -> windows::core::Result<Self> {
        let (notifier, class) = match &sender {
            Sender::Package => (
                ToastNotificationManager::CreateToastNotifier()?,
                PACKAGED_ACTIVATOR,
            ),
            Sender::Portable(plan) => {
                plan.register()?;
                let aumid = HSTRING::from(&plan.aumid);
                (
                    ToastNotificationManager::CreateToastNotifierWithId(&aumid)?,
                    plan.clsid,
                )
            }
        };
        let activator =
            ActivatorRegistration::register(GUID::from_u128(class), Arc::new(activated))?;
        Ok(Self {
            notifier,
            reads_setting: matches!(sender, Sender::Package),
            activator,
        })
    }

    /// Fails when Windows will not show it, such as when the user turned WorkTime's
    /// notifications off, where the .NET app gets no notification id. Without package identity,
    /// Windows drops such a toast silently instead.
    pub fn show(&self, request: &NotificationRequest) -> windows::core::Result<()> {
        if self.reads_setting && self.notifier.Setting()? != NotificationSetting::Enabled {
            return Err(Error::new(
                E_FAIL,
                "Windows did not accept the notification.",
            ));
        }
        let toast = ToastNotification::CreateToastNotification(&document(request)?)?;
        self.notifier.Show(&toast)
    }
}

fn document(request: &NotificationRequest) -> windows::core::Result<XmlDocument> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(request)))?;
    Ok(document)
}

#[cfg(test)]
mod tests {
    //! The real WinRT XML parser, which needs no package identity and shows nothing.

    use super::super::delivery_id;
    use super::*;

    #[test]
    fn windows_reads_the_toast_back_unchanged() {
        let request = NotificationRequest {
            delivery_id: "a=b;c%d".to_owned(),
            title: "Bientôt".to_owned(),
            body: r#"<b> & "quoted" 'text'"#.to_owned(),
        };

        let document = document(&request).unwrap();
        let texts = document
            .GetElementsByTagName(&HSTRING::from("text"))
            .unwrap();
        let toast = document.DocumentElement().unwrap();

        assert_eq!(texts.Length().unwrap(), 2);
        assert_eq!(texts.Item(0).unwrap().InnerText().unwrap(), "Bientôt");
        assert_eq!(
            texts.Item(1).unwrap().InnerText().unwrap(),
            r#"<b> & "quoted" 'text'"#
        );
        let launch = toast.GetAttribute(&HSTRING::from("launch")).unwrap();
        assert_eq!(launch, "deliveryId=a%3Db%3Bc%25d");
        assert_eq!(delivery_id(&launch.to_string()).as_deref(), Some("a=b;c%d"));
    }
}
