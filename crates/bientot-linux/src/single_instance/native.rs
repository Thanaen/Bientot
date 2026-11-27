//! The lock and the socket behind the single instance.

use std::fs::{DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bientot_platform::single_instance::Acquired;
use bientot_store::data_directory::DataDirectory;

use super::{InstanceNote, file_stem, instance_folder};

/// How long a later launch tries to reach the owner before it exits anyway, as if accepted. Its
/// request waits in the socket meanwhile, so an owner still starting shows itself once it
/// listens.
const ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a later launch waits before it looks at the folder again, while the owner is not
/// listening yet or is leaving.
const RETRY: Duration = Duration::from_millis(50);

/// Holds the data folder until dropped. Until it listens, a later launch that connects waits for
/// its answer.
pub struct PrimaryInstance {
    // Dropped before the lock, so that no later owner's socket is ever removed.
    socket: Option<Socket>,
    lock: File,
    /// What went wrong with the socket, told once it listens.
    notes: Vec<InstanceNote>,
}

/// The socket a later launch reaches the owner on; its file goes with it.
struct Socket {
    listener: UnixListener,
    path: PathBuf,
}

impl Drop for Socket {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

impl PrimaryInstance {
    /// Holds the folder whose files are `<stem>.lock` and `<stem>.sock` in `folder`, or hands
    /// this launch to the Bientôt that holds it.
    ///
    /// A launch that finds the owner gone, whether it left before or while this launch waited
    /// for its answer, holds the folder instead.
    pub fn claim(folder: &Path, stem: &str) -> io::Result<Acquired<Self>> {
        create_private(folder)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(folder.join(format!("{stem}.lock")))?;
        let socket = folder.join(format!("{stem}.sock"));
        let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
        loop {
            match lock.try_lock() {
                Ok(()) => {
                    let mut notes = Vec::new();
                    return Ok(Acquired::Primary(Self {
                        socket: bind(socket, &mut notes),
                        lock,
                        notes,
                    }));
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Ok(Acquired::Secondary);
            }
            // Refused or missing: the owner is not listening yet, or it is leaving.
            if let Ok(stream) = UnixStream::connect(&socket) {
                match ask(stream, deadline) {
                    Answer::Accepted | Answer::Unanswered => return Ok(Acquired::Secondary),
                    Answer::Broken => {}
                }
            }
            thread::sleep(until(deadline).min(RETRY));
        }
    }

    /// Holds `directory`, in the folder [`instance_folder`] names for this session.
    pub fn acquire(directory: &DataDirectory) -> io::Result<Acquired<Self>> {
        let folder = instance_folder(std::env::var_os("XDG_RUNTIME_DIR"), directory.path());
        Self::claim(&folder, file_stem(&directory.case_sensitive_instance_key()))
    }

    /// Calls `on_activation` on a thread of its own for each request of a later launch. The
    /// folder stays held until the listener is dropped. `on_note` is told at once what went
    /// wrong with the socket when the folder was claimed, then, on that thread, a failure to
    /// receive a launch.
    pub fn listen(
        self,
        mut on_activation: impl FnMut() + Send + 'static,
        mut on_note: impl FnMut(InstanceNote) + Send + 'static,
    ) -> io::Result<ActivationListener> {
        let Self {
            socket,
            lock,
            notes,
        } = self;
        notes.into_iter().for_each(&mut on_note);
        let stop = Arc::new(AtomicBool::new(false));
        let socket = socket.map(Arc::new);
        let thread = socket
            .as_ref()
            .map(|socket| {
                let (socket, stop) = (Arc::clone(socket), Arc::clone(&stop));
                thread::Builder::new()
                    .name("bientot-activation".into())
                    .spawn(move || {
                        while let Some(accepted) =
                            next_launch(&socket.listener, &stop, &mut on_note)
                        {
                            if accept(accepted) {
                                on_activation();
                            }
                        }
                    })
            })
            .transpose()?;
        Ok(ActivationListener {
            stop,
            thread,
            socket,
            _lock: lock,
        })
    }
}

/// Stops listening and releases the data folder when dropped, once an `on_activation` that runs
/// has returned. A launch the listener did not accept sees its connection break and takes the
/// folder over.
pub struct ActivationListener {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    // Dropped before the lock, as in `PrimaryInstance`.
    socket: Option<Arc<Socket>>,
    _lock: File,
}

impl Drop for ActivationListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let (Some(thread), Some(socket)) = (self.thread.take(), &self.socket) else {
            return;
        };
        // A connection of its own wakes the thread from `accept`. Should none get through, the
        // thread is left blocked there, and serves no one once woken.
        if UnixStream::connect(&socket.path).is_ok() {
            thread.join().ok();
        }
    }
}

/// The next connection of a later launch, `None` once the listener stops or `accept` fails,
/// which `on_note` is told.
fn next_launch(
    listener: &UnixListener,
    stop: &AtomicBool,
    on_note: &mut impl FnMut(InstanceNote),
) -> Option<UnixStream> {
    let accepted = listener.accept();
    if stop.load(Ordering::Acquire) {
        return None;
    }
    accepted
        .inspect_err(|error| on_note(InstanceNote::AcceptFailed(error.kind())))
        .ok()
        .map(|(stream, _)| stream)
}

/// Takes the request of a later launch on `stream` and accepts it on that connection; whether a
/// request came.
fn accept(mut stream: UnixStream) -> bool {
    let mut request = [0];
    if stream.set_read_timeout(Some(ACCEPTANCE_TIMEOUT)).is_err()
        || !matches!(stream.read(&mut request), Ok(1))
    {
        return false;
    }
    // The launch is owed an activation from here on, even if it is gone: it may have exited
    // unanswered at its deadline.
    stream.write_all(&[1]).ok();
    true
}

/// The owner's answer to a launch's request.
enum Answer {
    Accepted,
    /// The owner did not answer by the deadline.
    Unanswered,
    /// The owner closed the connection without accepting: it is leaving.
    Broken,
}

/// Sends this launch's request on `stream` and waits until `deadline` for the owner to accept
/// it on the same connection.
fn ask(mut stream: UnixStream, deadline: Instant) -> Answer {
    let timed_out = |error: &io::Error| {
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        )
    };
    let remaining = until(deadline);
    if remaining.is_zero()
        || stream.set_write_timeout(Some(remaining)).is_err()
        || stream.set_read_timeout(Some(remaining)).is_err()
    {
        return Answer::Unanswered;
    }
    match stream.write(&[1]) {
        Ok(1) => {}
        Err(error) if timed_out(&error) => return Answer::Unanswered,
        _ => return Answer::Broken,
    }
    let mut acceptance = [0];
    match stream.read(&mut acceptance) {
        Ok(1) => Answer::Accepted,
        Err(error) if timed_out(&error) => Answer::Unanswered,
        _ => Answer::Broken,
    }
}

/// Listens at `path` in place of the socket a dead owner left there, and adds to `notes` what
/// went wrong. Without a socket the folder is still held, and a later launch exits without
/// showing this one.
fn bind(path: PathBuf, notes: &mut Vec<InstanceNote>) -> Option<Socket> {
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            notes.push(InstanceNote::StaleSocketKept(error.kind()));
        }
        _ => {}
    }
    UnixListener::bind(&path)
        .inspect_err(|error| notes.push(InstanceNote::Unbound(error.kind())))
        .ok()
        .map(|listener| Socket { listener, path })
}

/// Creates `folder`, readable by this user only, and the folders above it as usual.
fn create_private(folder: &Path) -> io::Result<()> {
    if let Some(parent) = folder.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match DirBuilder::new().mode(0o700).create(folder) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
        _ => Ok(()),
    }
}

fn until(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

#[cfg(test)]
mod tests;
