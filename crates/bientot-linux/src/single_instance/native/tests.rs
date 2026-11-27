use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;

use tempfile::TempDir;

use super::*;

const STEM: &str = "WorkTime-0123456789abcdef0123456789abcdef";
const SOON: Duration = Duration::from_secs(5);

fn claim(folder: &Path) -> Acquired<PrimaryInstance> {
    PrimaryInstance::claim(folder, STEM).unwrap()
}

fn primary(folder: &Path) -> PrimaryInstance {
    match claim(folder) {
        Acquired::Primary(instance) => instance,
        Acquired::Secondary => panic!("the folder is held"),
    }
}

fn is_secondary(acquired: &Acquired<PrimaryInstance>) -> bool {
    matches!(acquired, Acquired::Secondary)
}

/// Listens on `instance` and sends a value for each activation.
fn listen(instance: PrimaryInstance) -> (ActivationListener, mpsc::Receiver<()>) {
    let (listener, activations, _notes) = listen_noting(instance);
    (listener, activations)
}

/// Listens on `instance`, and sends a value for each activation and each note.
fn listen_noting(
    instance: PrimaryInstance,
) -> (
    ActivationListener,
    mpsc::Receiver<()>,
    mpsc::Receiver<InstanceNote>,
) {
    let (activated, activations) = mpsc::channel();
    let (noted, notes) = mpsc::channel();
    let listener = instance
        .listen(
            move || {
                activated.send(()).ok();
            },
            move |note| {
                noted.send(note).ok();
            },
        )
        .unwrap();
    (listener, activations, notes)
}

#[test]
fn a_later_launch_hands_over_to_the_owner_and_is_secondary() {
    let folder = TempDir::new().unwrap();
    let (_listener, activations, notes) = listen_noting(primary(folder.path()));

    for _ in 0..2 {
        let started = Instant::now();
        assert!(is_secondary(&claim(folder.path())));
        assert!(
            started.elapsed() < ACCEPTANCE_TIMEOUT,
            "answered, not timed out"
        );
        activations
            .recv_timeout(SOON)
            .expect("the owner is activated");
    }
    assert!(activations.try_recv().is_err(), "once per launch");
    assert_eq!(notes.try_recv().ok(), None, "nothing went wrong");
}

#[test]
fn a_launch_before_the_owner_listens_waits_for_its_answer() {
    let folder = TempDir::new().unwrap();
    let owner = primary(folder.path());
    let later = {
        let folder = folder.path().to_owned();
        thread::spawn(move || is_secondary(&claim(&folder)))
    };
    thread::sleep(Duration::from_millis(300));
    let (_listener, activations) = listen(owner);
    assert!(later.join().unwrap());
    activations.recv_timeout(SOON).unwrap();
}

#[test]
fn an_owner_that_never_answers_still_shows_itself_once_it_listens() {
    let folder = TempDir::new().unwrap();
    let owner = primary(folder.path());
    let started = Instant::now();
    assert!(is_secondary(&claim(folder.path())));
    assert!(started.elapsed() >= ACCEPTANCE_TIMEOUT);

    let (_listener, activations) = listen(owner);
    activations
        .recv_timeout(SOON)
        .expect("the request waited in the socket");
}

#[test]
fn a_dropped_listener_releases_the_folder_and_removes_its_socket() {
    let folder = TempDir::new().unwrap();
    let (listener, _activations) = listen(primary(folder.path()));
    let socket = folder.path().join(format!("{STEM}.sock"));
    assert!(socket.exists());

    let started = Instant::now();
    drop(listener);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "no wait to stop"
    );
    assert!(!socket.exists());
    let (_listener, activations) = listen(primary(folder.path()));
    assert!(is_secondary(&claim(folder.path())));
    activations.recv_timeout(SOON).unwrap();
}

#[test]
fn an_instance_dropped_before_it_listens_releases_the_folder() {
    let folder = TempDir::new().unwrap();
    drop(primary(folder.path()));
    assert!(!folder.path().join(format!("{STEM}.sock")).exists());
    primary(folder.path());
}

#[test]
fn the_socket_a_killed_owner_left_is_replaced() {
    let folder = TempDir::new().unwrap();
    let socket = folder.path().join(format!("{STEM}.sock"));
    // What `kill -9` leaves: the socket's file, with no one listening, and the lock released.
    drop(UnixListener::bind(&socket).unwrap());
    std::fs::write(folder.path().join(format!("{STEM}.lock")), "").unwrap();
    assert!(UnixStream::connect(&socket).is_err(), "stale");

    let (_listener, activations, notes) = listen_noting(primary(folder.path()));
    assert!(is_secondary(&claim(folder.path())));
    activations.recv_timeout(SOON).unwrap();
    assert_eq!(notes.try_recv().ok(), None, "a stale socket is no failure");
}

#[test]
fn a_stale_socket_that_cannot_be_removed_is_noted_and_leaves_no_socket() {
    let folder = TempDir::new().unwrap();
    let socket = folder.path().join(format!("{STEM}.sock"));
    std::fs::create_dir(&socket).unwrap();
    std::fs::write(socket.join("kept"), "").unwrap();

    let (_listener, activations, notes) = listen_noting(primary(folder.path()));
    assert_eq!(
        notes.try_iter().collect::<Vec<_>>(),
        [
            InstanceNote::StaleSocketKept(io::ErrorKind::IsADirectory),
            InstanceNote::Unbound(io::ErrorKind::AddrInUse),
        ]
    );
    assert!(is_secondary(&claim(folder.path())));
    assert!(activations.try_recv().is_err());
}

#[test]
fn a_launch_takes_over_from_an_owner_that_leaves_while_it_waits() {
    let folder = TempDir::new().unwrap();
    let owner = primary(folder.path());
    let later = {
        let folder = folder.path().to_owned();
        thread::spawn(move || matches!(claim(&folder), Acquired::Primary(_)))
    };
    thread::sleep(Duration::from_millis(300));
    drop(owner);
    assert!(later.join().unwrap());
}

#[test]
fn the_folder_is_private_to_the_user_and_made_under_a_data_folder_not_yet_there() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("Bientot");
    let folder = instance_folder(None, &data);
    let _held = primary(&folder);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&folder), 0o700);
    assert_ne!(mode(&data), 0o700, "the data folder keeps the usual mode");
    assert!(folder.join(format!("{STEM}.lock")).exists());
}

#[test]
fn without_a_socket_the_folder_is_still_held() {
    let root = TempDir::new().unwrap();
    let folder = root.path().join("a".repeat(120));
    let (_listener, activations, notes) = listen_noting(primary(&folder));
    assert_eq!(
        notes.try_iter().collect::<Vec<_>>(),
        [InstanceNote::Unbound(io::ErrorKind::InvalidInput)]
    );
    assert!(is_secondary(&claim(&folder)));
    assert!(activations.try_recv().is_err());
}
