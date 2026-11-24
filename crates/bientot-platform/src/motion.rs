//! Whether the user asked the system for less motion.

use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;

/// Called after a change of the setting, on whatever thread the system tells it.
pub type OnChange = Box<dyn Fn() + Send + Sync>;

pub trait ReducedMotion {
    /// Stops telling the changes when dropped.
    type Watch;

    /// Whether animations should show at rest.
    fn current(&self) -> bool;

    /// Calls `on_change` after each change of the setting, which [`Self::current`] then reads.
    fn watch(&self, on_change: OnChange) -> io::Result<Self::Watch>;
}

/// A setting the caller changes with [`Self::set`]. Motion is not reduced at first.
#[derive(Clone, Default)]
pub struct InMemoryMotion {
    reduced: Rc<Cell<bool>>,
    watchers: Rc<RefCell<Vec<OnChange>>>,
}

impl InMemoryMotion {
    pub fn set(&self, reduced: bool) {
        self.reduced.set(reduced);
        for on_change in self.watchers.borrow().iter() {
            on_change();
        }
    }
}

impl ReducedMotion for InMemoryMotion {
    type Watch = ();

    fn current(&self) -> bool {
        self.reduced.get()
    }

    fn watch(&self, on_change: OnChange) -> io::Result<()> {
        self.watchers.borrow_mut().push(on_change);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_watcher_hears_each_change_and_reads_it_afterwards() {
        let motion = InMemoryMotion::default();
        let heard = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&heard);
        motion
            .watch(Box::new(move || {
                counted.fetch_add(1, Ordering::Relaxed);
            }))
            .unwrap();
        assert!(!motion.current());

        motion.set(true);

        assert_eq!(heard.load(Ordering::Relaxed), 1);
        assert!(motion.current());
    }
}
