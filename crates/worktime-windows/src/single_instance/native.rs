//! The named pipe behind the single instance.

#![allow(unsafe_code)]

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, FALSE, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, TRUE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcessId, INFINITE, SetEvent, WaitForMultipleObjects,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows_sys::core::BOOL;
use worktime_store::data_directory::DataDirectory;

use super::pipe_name;

/// How long a later launch tries to reach the owner before it exits anyway. An owner that has
/// not answered by then is taken as hung, and the launch exits as if it had been accepted.
const ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a later launch waits for a busy owner, or for a leaving one to be gone, before it
/// looks at the folder again.
const RETRY: Duration = Duration::from_millis(50);

pub enum Instance {
    /// This launch owns the folder.
    Primary(PrimaryInstance),
    /// Another WorkTime owns the folder and accepted this launch's request to show itself; this
    /// launch should exit.
    Secondary,
}

/// Owns the data directory until dropped: the only instance of the folder's pipe.
///
/// Until it listens, a later launch that connects waits for its answer, as the .NET app queues
/// redirects that arrive before it registers its handler.
pub struct PrimaryInstance {
    pipe: OwnedHandle,
}

/// Claims `directory` for this launch, or hands the launch to the WorkTime that owns it.
///
/// A launch that finds the owner gone, whether it left before or while this launch waited for
/// its answer, owns the folder instead. It owes itself no request: the host shows the window of a
/// primary launch anyway.
pub fn claim(directory: &DataDirectory) -> io::Result<Instance> {
    let name = wide(&pipe_name(directory, session()?));
    let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
    loop {
        if let Some(pipe) = own(&name)? {
            return Ok(Instance::Primary(PrimaryInstance { pipe }));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(Instance::Secondary);
        }
        match connect(&name)? {
            Connection::Open(pipe) => match ask(&pipe, deadline)? {
                Answer::Accepted | Answer::Unanswered => return Ok(Instance::Secondary),
                Answer::Broken => {}
            },
            Connection::Busy => {
                // SAFETY: the name is a NUL-terminated UTF-16 string borrowed for the call. A
                // timeout or a vanished pipe only sends this launch round the loop again.
                unsafe { WaitNamedPipeW(name.as_ptr(), millis(remaining.min(RETRY)).max(1)) };
            }
            Connection::Gone => thread::sleep(remaining.min(RETRY)),
        }
    }
}

impl PrimaryInstance {
    /// Calls `on_activation` on a background thread for each request of a later launch. The
    /// folder stays owned until the listener is dropped.
    pub fn listen(
        self,
        mut on_activation: impl FnMut() + Send + 'static,
    ) -> io::Result<ActivationListener> {
        let stop = Arc::new(new_event()?);
        let pipe = Arc::new(Pipe(Mutex::new(Some(self.pipe))));
        let thread_stop = Arc::clone(&stop);
        let release_on_exit = ReleaseOnExit(Arc::clone(&pipe));
        let thread = thread::Builder::new()
            .name("worktime-activation".into())
            .spawn(move || {
                let pipe = &release_on_exit.0;
                loop {
                    // The pipe is locked while a connection is served, not while `on_activation`
                    // runs, so that a drop can release the folder during a slow activation.
                    let served = match pipe.lock().as_ref() {
                        Some(pipe) => serve(pipe, &thread_stop),
                        None => return,
                    };
                    match served {
                        Ok(Served::Accepted) => on_activation(),
                        Ok(Served::Dropped) => {}
                        Ok(Served::Stopped) | Err(_) => return,
                    }
                }
            })?;
        Ok(ActivationListener {
            stop,
            pipe,
            thread: Some(thread),
        })
    }
}

/// Stops listening and releases the data directory when dropped, as the .NET app unregisters
/// its key before restarting in another language.
///
/// Dropping it interrupts the listener, closes the pipe, then waits for an `on_activation` that
/// is running. A launch is accepted only on its own connection, by a listener that then calls
/// `on_activation` for it; a launch the listener did not accept sees its connection break and
/// takes the folder over, even while that last `on_activation` runs.
pub struct ActivationListener {
    stop: Arc<OwnedHandle>,
    pipe: Arc<Pipe>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ActivationListener {
    fn drop(&mut self) {
        set(&self.stop).ok();
        self.pipe.release();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The folder's pipe, shared by the listener and its thread; `None` once the folder is released.
struct Pipe(Mutex<Option<OwnedHandle>>);

impl Pipe {
    fn lock(&self) -> MutexGuard<'_, Option<OwnedHandle>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Closes the pipe, which releases the folder and breaks the connection of any launch still
    /// waiting for an answer.
    fn release(&self) {
        drop(self.lock().take());
    }
}

/// Releases the folder when the listening thread ends, whether it stops, fails or panics in
/// `on_activation`, so that the folder never looks owned by a WorkTime that stopped listening.
struct ReleaseOnExit(Arc<Pipe>);

impl Drop for ReleaseOnExit {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// What became of one connection to the owner.
enum Served {
    /// The launch was accepted on its connection and is owed an activation.
    Accepted,
    /// The launch left or sent no request; nothing is owed.
    Dropped,
    /// The listener was told to stop before it accepted anyone.
    Stopped,
}

/// Waits for a later launch on `pipe`, takes its request and accepts it on its own connection.
fn serve(pipe: &OwnedHandle, stop: &OwnedHandle) -> io::Result<Served> {
    // SAFETY: `pipe` is a valid server handle, and `overlapped` keeps the `OVERLAPPED` in place
    // until the operation is over.
    let connect = |handle, overlapped| unsafe { ConnectNamedPipe(handle, overlapped) };
    match overlapped(pipe, Some(stop), None, connect)? {
        Outcome::Done(Ok(_)) => {}
        Outcome::Done(Err(error)) if is(&error, ERROR_PIPE_CONNECTED) => {}
        // A launch connected and left before the listener looked.
        Outcome::Done(Err(error)) if is(&error, ERROR_NO_DATA) => return disconnect(pipe),
        Outcome::Done(Err(error)) => return Err(error),
        Outcome::Stopped | Outcome::TimedOut => return Ok(Served::Stopped),
    }
    let mut request = 0;
    match overlapped(
        pipe,
        Some(stop),
        Some(ACCEPTANCE_TIMEOUT),
        read(&mut request),
    )? {
        Outcome::Done(Ok(1)) => {}
        Outcome::Stopped => return Ok(Served::Stopped),
        Outcome::Done(_) | Outcome::TimedOut => return disconnect(pipe),
    }
    // The launch is owed an activation from here on, even if it is gone: it may have exited
    // unanswered at its deadline. Errors no longer matter.
    let acceptance = 1;
    let _ = overlapped(pipe, None, Some(ACCEPTANCE_TIMEOUT), write(&acceptance));
    // Disconnecting discards an acceptance the launch has not read yet, so the listener waits
    // for the launch to close its end. Stopping skips that, leaving the acceptance to be read
    // after the pipe closes.
    let mut rest = 0;
    let closed = overlapped(pipe, Some(stop), Some(ACCEPTANCE_TIMEOUT), read(&mut rest));
    if !matches!(closed, Ok(Outcome::Stopped)) {
        let _ = disconnect(pipe);
    }
    Ok(Served::Accepted)
}

fn disconnect(pipe: &OwnedHandle) -> io::Result<Served> {
    // SAFETY: `pipe` is a valid server handle with no operation in progress.
    if unsafe { DisconnectNamedPipe(pipe.as_raw_handle()) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(Served::Dropped)
}

/// Creates the folder's pipe, or returns `None` when another WorkTime holds it.
fn own(name: &[u16]) -> io::Result<Option<OwnedHandle>> {
    // SAFETY: the name is a NUL-terminated UTF-16 string borrowed for the call. Null attributes
    // give the pipe the default security, full control for this user and read access for others;
    // remote clients are refused, and a single instance leaves no one room to add another.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1,
            1,
            0,
            ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        // `FILE_FLAG_FIRST_PIPE_INSTANCE` refuses access when the pipe exists; a pipe whose only
        // instance is still closing may answer busy instead.
        return if is(&error, ERROR_ACCESS_DENIED) || is(&error, ERROR_PIPE_BUSY) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    Ok(Some(owned(handle)))
}

/// What a later launch found when it connected to the owner.
enum Connection {
    Open(OwnedHandle),
    /// The owner is serving another launch, running an activation, or leaving.
    Busy,
    /// The owner left after this launch looked.
    Gone,
}

fn connect(name: &[u16]) -> io::Result<Connection> {
    // SAFETY: the name is a NUL-terminated UTF-16 string borrowed for the call; the flags let the
    // owner identify this launch but not act as it.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_NONE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        return if is(&error, ERROR_PIPE_BUSY) {
            Ok(Connection::Busy)
        } else if is(&error, ERROR_FILE_NOT_FOUND) {
            Ok(Connection::Gone)
        } else {
            Err(error)
        };
    }
    Ok(Connection::Open(owned(handle)))
}

/// The owner's answer to a launch's request.
enum Answer {
    Accepted,
    /// The owner did not answer by the deadline.
    Unanswered,
    /// The owner closed the connection without accepting: it is leaving.
    Broken,
}

/// Sends this launch's request on `pipe` and waits until `deadline` for the owner to accept it
/// on the same connection.
fn ask(pipe: &OwnedHandle, deadline: Instant) -> io::Result<Answer> {
    // This launch holds the foreground right; pass it on so the owner can show its window. The
    // owner's process id is unknown here, unlike the .NET redirect, so any process may take it
    // until the next input.
    // SAFETY: takes a process id and touches no memory. A refusal only leaves the window behind.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let request = 1;
    match overlapped(pipe, None, Some(until(deadline)), write(&request))? {
        Outcome::Done(Ok(1)) => {}
        Outcome::Done(_) => return Ok(Answer::Broken),
        Outcome::Stopped | Outcome::TimedOut => return Ok(Answer::Unanswered),
    }
    let mut acceptance = 0;
    Ok(
        match overlapped(pipe, None, Some(until(deadline)), read(&mut acceptance))? {
            Outcome::Done(Ok(1)) => Answer::Accepted,
            Outcome::Done(_) => Answer::Broken,
            Outcome::Stopped | Outcome::TimedOut => Answer::Unanswered,
        },
    )
}

/// How an overlapped operation ended.
enum Outcome {
    /// The operation finished, with the number of bytes it moved or its error.
    Done(io::Result<u32>),
    /// The stop event was set first, and the operation was cancelled.
    Stopped,
    /// The timeout passed first, and the operation was cancelled.
    TimedOut,
}

/// Starts an overlapped operation on `handle` with `start`, then waits for it, for `stop` or for
/// `timeout`, whichever comes first. The operation is over when this returns, so the buffers it
/// borrowed and its `OVERLAPPED` can go.
fn overlapped(
    handle: &OwnedHandle,
    stop: Option<&OwnedHandle>,
    timeout: Option<Duration>,
    start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> BOOL,
) -> io::Result<Outcome> {
    let event = new_event()?;
    let mut overlapped = OVERLAPPED {
        hEvent: event.as_raw_handle(),
        ..Default::default()
    };
    let handle = handle.as_raw_handle();
    if start(handle, &mut overlapped) != FALSE {
        // SAFETY: the operation finished as it started; `overlapped` is the one it used.
        return Ok(Outcome::Done(unsafe { finished(handle, &overlapped) }));
    }
    let error = io::Error::last_os_error();
    if !is(&error, ERROR_IO_PENDING) {
        return Ok(Outcome::Done(Err(error)));
    }
    // Stop comes first, so that a stopping listener takes nothing more.
    let waits: Vec<HANDLE> = stop
        .map(AsRawHandle::as_raw_handle)
        .into_iter()
        .chain([event.as_raw_handle()])
        .collect();
    let done = WAIT_OBJECT_0 + waits.len() as u32 - 1;
    // SAFETY: `waits` holds one or two valid handles, open for the whole wait.
    let signalled = unsafe {
        WaitForMultipleObjects(
            waits.len() as u32,
            waits.as_ptr(),
            FALSE,
            timeout.map_or(INFINITE, millis),
        )
    };
    let outcome = match signalled {
        _ if signalled == done => {
            // SAFETY: the operation is over, since its event is set; `overlapped` is the one it
            // used.
            return Ok(Outcome::Done(unsafe { finished(handle, &overlapped) }));
        }
        WAIT_TIMEOUT => Ok(Outcome::TimedOut),
        WAIT_OBJECT_0 => Ok(Outcome::Stopped),
        _ => Err(io::Error::last_os_error()),
    };
    let mut moved = 0;
    // SAFETY: `overlapped` names the operation started above; cancelling one that finished in the
    // meantime does nothing. Waiting for the result keeps `overlapped` and the buffers in place
    // until the operation is over.
    unsafe {
        CancelIoEx(handle, &overlapped);
        GetOverlappedResult(handle, &overlapped, &mut moved, TRUE);
    }
    outcome
}

/// The bytes a finished overlapped operation moved, or its error.
///
/// # Safety
///
/// The operation `overlapped` describes on `handle` must be over.
unsafe fn finished(handle: HANDLE, overlapped: &OVERLAPPED) -> io::Result<u32> {
    let mut moved = 0;
    // SAFETY: the caller guarantees the operation is over, so this does not wait.
    if unsafe { GetOverlappedResult(handle, overlapped, &mut moved, FALSE) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(moved)
}

/// Reads one byte into `byte`, for [`overlapped`].
fn read(byte: &mut u8) -> impl FnOnce(HANDLE, *mut OVERLAPPED) -> BOOL + '_ {
    // SAFETY: `byte` stays borrowed until `overlapped` returns, once the operation is over, and
    // overlapped reads report their size through `OVERLAPPED`.
    move |handle, overlapped| unsafe { ReadFile(handle, byte, 1, ptr::null_mut(), overlapped) }
}

/// Writes `byte`, for [`overlapped`].
fn write(byte: &u8) -> impl FnOnce(HANDLE, *mut OVERLAPPED) -> BOOL + '_ {
    // SAFETY: `byte` stays borrowed until `overlapped` returns, once the operation is over, and
    // overlapped writes report their size through `OVERLAPPED`.
    move |handle, overlapped| unsafe { WriteFile(handle, byte, 1, ptr::null_mut(), overlapped) }
}

/// The sign-in session of this process, which scopes the pipe's name.
fn session() -> io::Result<u32> {
    let mut session = 0;
    // SAFETY: `session` is a valid place for the id, borrowed for the call.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(session)
}

fn is(error: &io::Error, code: u32) -> bool {
    error.raw_os_error() == Some(code as i32)
}

fn until(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// `duration` in milliseconds for a wait, short of `INFINITE`.
fn millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).map_or(INFINITE - 1, |millis| millis.min(INFINITE - 1))
}

fn owned(handle: HANDLE) -> OwnedHandle {
    // SAFETY: `handle` is a new, valid handle that nothing else owns or closes.
    unsafe { OwnedHandle::from_raw_handle(handle) }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// An unnamed, unsignalled, manual-reset event.
fn new_event() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and name create a private event.
    let handle = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(owned(handle))
}

fn set(event: &OwnedHandle) -> io::Result<()> {
    // SAFETY: `event` is a valid event handle owned for the call.
    if unsafe { SetEvent(event.as_raw_handle()) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
