//! One Bientôt per data folder: a later launch on a folder another Bientôt holds asks that one to
//! show itself, then exits.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use bientot_store::data_directory::DataDirectory;

/// What a launch got when it asked for a data folder.
pub enum Acquired<L> {
    /// This launch holds the folder.
    Primary(L),
    /// Another Bientôt holds the folder and accepted this launch's request to show itself; this
    /// launch should exit.
    Secondary,
}

/// The hold of this launch on its data folder.
pub trait InstanceLock: Sized {
    /// Holds the folder and receives the later launches until dropped.
    type Listener;

    fn acquire(directory: &DataDirectory) -> io::Result<Acquired<Self>>;

    /// Calls `on_activation` for each later launch on the folder. Until then, a later launch
    /// waits for its answer.
    fn listen(self, on_activation: impl FnMut() + Send + 'static) -> io::Result<Self::Listener>;
}

type OnActivation = Box<dyn FnMut() + Send>;

/// The folders held in this process, and how each holder is shown.
static HELD: Mutex<BTreeMap<String, Option<OnActivation>>> = Mutex::new(BTreeMap::new());

fn held() -> MutexGuard<'static, BTreeMap<String, Option<OnActivation>>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A hold that only this process sees: a later acquire of a held folder calls the holder's
/// `on_activation` on the acquiring thread, when it listens, and is [`Acquired::Secondary`].
pub struct InMemoryInstance(Hold);

/// Keeps an [`InMemoryInstance`] folder held while it listens.
pub struct InMemoryListener {
    _hold: Hold,
}

/// Releases the folder when dropped.
struct Hold(String);

impl Drop for Hold {
    fn drop(&mut self) {
        held().remove(&self.0);
    }
}

impl InstanceLock for InMemoryInstance {
    type Listener = InMemoryListener;

    fn acquire(directory: &DataDirectory) -> io::Result<Acquired<Self>> {
        let key = directory.instance_key();
        let mut held = held();
        if let Some(holder) = held.get_mut(&key) {
            if let Some(on_activation) = holder {
                on_activation();
            }
            return Ok(Acquired::Secondary);
        }
        held.insert(key.clone(), None);
        Ok(Acquired::Primary(Self(Hold(key))))
    }

    fn listen(self, on_activation: impl FnMut() + Send + 'static) -> io::Result<InMemoryListener> {
        held().insert(self.0.0.clone(), Some(Box::new(on_activation)));
        Ok(InMemoryListener { _hold: self.0 })
    }
}

#[cfg(test)]
mod tests;
