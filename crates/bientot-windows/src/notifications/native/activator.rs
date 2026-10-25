//! The COM activator through which Windows hands WorkTime the clicks on its toasts. The class is
//! the manifest's `ToastActivatorCLSID` for the MSIX build and the `CustomActivator` the portable
//! build registers, and a toast kept in Notification Center reaches the app only through it:
//! Windows calls `INotificationActivationCallback::Activate` on the class object the running app
//! registered, or starts the class's COM server when none is registered and calls the class
//! object the new process registers.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_FAIL};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx, CoRegisterClassObject,
    CoRevokeClassObject, CoUninitialize, IClassFactory, IClassFactory_Impl, REGCLS_MULTIPLEUSE,
};
use windows::Win32::UI::Notifications::{
    INotificationActivationCallback, INotificationActivationCallback_Impl,
    NOTIFICATION_USER_INPUT_DATA,
};
use windows::core::{BOOL, Error, GUID, IUnknown, Interface, PCWSTR, Ref, implement};

use super::super::delivery_id;

/// Called with the clicked toast's delivery id, if it carries one. COM may call it on several of
/// its RPC threads at once, hence `Send + Sync`.
pub(super) type Activated = Arc<dyn Fn(Option<String>) + Send + Sync>;

/// The class object registered with COM until dropped.
///
/// It lives on a thread of its own that joins the multithreaded apartment. Registered from there,
/// COM calls `Activate` on its RPC threads, so the callback never waits for the UI thread, whose
/// apartment WorkTime does not choose, to pump messages. The same thread revokes the class and
/// leaves the apartment, as COM requires.
pub(super) struct ActivatorRegistration {
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl ActivatorRegistration {
    pub(super) fn register(class: GUID, activated: Activated) -> windows::core::Result<Self> {
        let (registered, registration) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("worktime-toast-activator".to_owned())
            .spawn(move || serve(class, activated, &registered, &stopped))
            .map_err(|error| Error::new(E_FAIL, error.to_string()))?;
        match registration.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(Error::new(E_FAIL, "The toast activator thread stopped.")),
        }
    }
}

impl Drop for ActivatorRegistration {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    class: GUID,
    activated: Activated,
    registered: &SyncSender<windows::core::Result<()>>,
    stopped: &Receiver<()>,
) {
    // SAFETY: the first COM call of this new thread, balanced by `CoUninitialize` below.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        let _ = registered.send(Err(error));
        return;
    }
    let factory: IUnknown = Activator { activated }.into();
    // SAFETY: `class` and `factory` live through the call, and COM keeps its own reference to
    // `factory` until the class is revoked.
    match unsafe {
        CoRegisterClassObject(&class, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)
    } {
        Ok(cookie) => {
            let _ = registered.send(Ok(()));
            let _ = stopped.recv();
            // SAFETY: the cookie of the registration above, revoked once from its apartment.
            let _ = unsafe { CoRevokeClassObject(cookie) };
        }
        Err(error) => {
            let _ = registered.send(Err(error));
        }
    }
    drop(factory);
    // SAFETY: balances the `CoInitializeEx` above on the same thread, after its last COM call.
    unsafe { CoUninitialize() };
}

/// Both the class object COM asks for an instance and the instance it calls.
#[implement(IClassFactory, INotificationActivationCallback)]
struct Activator {
    activated: Activated,
}

impl IClassFactory_Impl for Activator_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        interface: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let instance: IUnknown = Activator {
            activated: Arc::clone(&self.activated),
        }
        .into();
        // SAFETY: COM passes the IID it wants and a writable out pointer, which `query` fills
        // with an owned reference or nulls.
        unsafe { instance.query(interface, object) }.ok()
    }

    /// WorkTime runs until the user quits it, so COM's lock count has no say in that.
    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

impl INotificationActivationCallback_Impl for Activator_Impl {
    /// Windows passes the toast's `launch` arguments as `invokedArgs`, or null when it has none.
    fn Activate(
        &self,
        _app_user_model_id: &PCWSTR,
        invoked_args: &PCWSTR,
        _data: *const NOTIFICATION_USER_INPUT_DATA,
        _count: u32,
    ) -> windows::core::Result<()> {
        let arguments = (!invoked_args.is_null()).then(|| {
            // SAFETY: a non-null `invokedArgs` is a NUL-terminated string COM keeps alive for
            // the call.
            String::from_utf16_lossy(unsafe { invoked_args.as_wide() })
        });
        (self.activated)(arguments.as_deref().and_then(delivery_id));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Registers a class of its own, so an installed WorkTime neither answers nor loses a click.

    use windows::Win32::Foundation::REGDB_E_CLASSNOTREG;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::core::w;

    use super::*;

    const TEST_ACTIVATOR: GUID = GUID::from_u128(0x3f0d6a52_1c7e_4b9a_9d41_8e2b5c7a0f13);

    #[test]
    fn the_registered_activator_hands_a_click_its_delivery_id_until_dropped() {
        let (clicked, clicks) = mpsc::channel();
        let registration = ActivatorRegistration::register(
            TEST_ACTIVATOR,
            Arc::new(move |delivery_id| clicked.send(delivery_id).unwrap()),
        )
        .unwrap();
        // SAFETY: joins this test thread to the apartment of the registration, balanced below.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .unwrap();

        // A class registered for a local server with `REGCLS_MULTIPLEUSE` also serves this
        // process in-process, which hands back the object itself rather than a proxy.
        // SAFETY: the CLSID is a live constant and no outer object aggregates the instance.
        let activator: INotificationActivationCallback =
            unsafe { CoCreateInstance(&TEST_ACTIVATOR, None, CLSCTX_INPROC_SERVER) }.unwrap();
        // SAFETY: both strings are NUL-terminated literals and no input data is passed.
        unsafe { activator.Activate(w!("WorkTime"), w!("deliveryId=a%3Db%3Bc%25d"), &[]) }.unwrap();
        // SAFETY: as above, with no arguments, as for a toast without `launch`.
        unsafe { activator.Activate(w!("WorkTime"), PCWSTR::null(), &[]) }.unwrap();
        drop(activator);

        assert_eq!(clicks.try_recv(), Ok(Some("a=b;c%d".to_owned())));
        assert_eq!(clicks.try_recv(), Ok(None));

        drop(registration);
        // SAFETY: as above.
        let revoked = unsafe {
            CoCreateInstance::<_, INotificationActivationCallback>(
                &TEST_ACTIVATOR,
                None,
                CLSCTX_INPROC_SERVER,
            )
        };
        assert_eq!(revoked.unwrap_err().code(), REGDB_E_CLASSNOTREG);
        // SAFETY: balances this thread's `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
}
