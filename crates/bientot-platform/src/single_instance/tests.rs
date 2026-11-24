use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A folder no other test asks for: the holds are shared by the whole test process.
fn folder(name: &str) -> DataDirectory {
    DataDirectory::from(PathBuf::from(format!("bientot-in-memory-instance-{name}")))
}

fn is_secondary(acquired: io::Result<Acquired<InMemoryInstance>>) -> bool {
    matches!(acquired, Ok(Acquired::Secondary))
}

#[test]
fn a_held_folder_turns_a_later_launch_away_until_it_is_released() {
    let directory = folder("released");
    let Ok(Acquired::Primary(first)) = InMemoryInstance::acquire(&directory) else {
        panic!("the first launch must hold the folder");
    };

    assert!(is_secondary(InMemoryInstance::acquire(&directory)));
    drop(first);
    assert!(matches!(
        InMemoryInstance::acquire(&directory),
        Ok(Acquired::Primary(_))
    ));
}

#[test]
fn a_later_launch_shows_the_listening_holder_and_the_listener_keeps_the_folder() {
    let directory = folder("listening");
    let Ok(Acquired::Primary(first)) = InMemoryInstance::acquire(&directory) else {
        panic!("the first launch must hold the folder");
    };
    let shown = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&shown);
    let listener = first
        .listen(move || {
            counted.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();

    assert!(is_secondary(InMemoryInstance::acquire(&directory)));
    assert!(is_secondary(InMemoryInstance::acquire(&directory)));
    assert_eq!(shown.load(Ordering::Relaxed), 2);
    drop(listener);
    assert!(matches!(
        InMemoryInstance::acquire(&directory),
        Ok(Acquired::Primary(_))
    ));
}
