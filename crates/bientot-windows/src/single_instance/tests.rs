use std::path::PathBuf;

use super::*;

#[test]
fn the_pipe_is_named_by_the_instance_key_and_the_session() {
    let directory = DataDirectory::from(PathBuf::from(r"c:\users\ana\appdata\local\worktime\"));

    assert_eq!(
        pipe_name(&directory, 3),
        r"\\.\pipe\WorkTime-b87383584653d36e172bb9f916813c99af6c1b873c5c84681d930cffdac58692-3"
    );
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);
    const SETTLE: Duration = Duration::from_millis(200);
    /// Well short of the five seconds a launch waits for its answer.
    const PROMPTLY: Duration = Duration::from_secs(2);

    fn folder() -> (tempfile::TempDir, DataDirectory) {
        let folder = tempfile::tempdir().unwrap();
        let directory = DataDirectory::from(folder.path().to_path_buf());
        (folder, directory)
    }

    fn primary(directory: &DataDirectory) -> PrimaryInstance {
        match claim(directory) {
            Ok(Instance::Primary(primary)) => primary,
            Ok(Instance::Secondary) => panic!("this launch must own the folder"),
            Err(error) => panic!("the claim failed: {error}"),
        }
    }

    fn launch(directory: &DataDirectory) -> JoinHandle<io::Result<Instance>> {
        let directory = directory.clone();
        thread::spawn(move || claim(&directory))
    }

    fn is_secondary(claimed: io::Result<Instance>) -> bool {
        matches!(claimed, Ok(Instance::Secondary))
    }

    /// A listener whose activations are counted and held until `resume` is dropped.
    fn held_listener(
        owner: PrimaryInstance,
    ) -> (ActivationListener, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, entries) = mpsc::channel();
        let (resume, resumed) = mpsc::channel::<()>();
        let listener = owner
            .listen(move || {
                entered.send(()).unwrap();
                let _ = resumed.recv();
            })
            .unwrap();
        (listener, entries, resume)
    }

    #[test]
    fn a_second_launch_activates_the_owner_until_it_releases_the_folder() {
        let (_folder, directory) = folder();
        let (activated, activations) = mpsc::channel();
        let listener = primary(&directory)
            .listen(move || activated.send(()).unwrap())
            .unwrap();

        assert!(is_secondary(claim(&directory)));
        activations.recv_timeout(WAIT).unwrap();
        assert!(is_secondary(claim(&directory)));
        activations.recv_timeout(WAIT).unwrap();
        drop(listener);

        assert!(matches!(claim(&directory), Ok(Instance::Primary(_))));
    }

    #[test]
    fn each_accepted_launch_is_one_activation() {
        let (_folder, directory) = folder();
        let (activated, activations) = mpsc::channel();
        let _listener = primary(&directory)
            .listen(move || activated.send(()).unwrap())
            .unwrap();

        let launches: Vec<_> = (0..8).map(|_| launch(&directory)).collect();
        let accepted = launches
            .into_iter()
            .map(|launch| is_secondary(launch.join().unwrap()))
            .filter(|&secondary| secondary)
            .count();

        assert_eq!(accepted, 8);
        for _ in 0..8 {
            activations.recv_timeout(WAIT).unwrap();
        }
        assert!(activations.recv_timeout(SETTLE).is_err());
    }

    #[test]
    fn a_request_sent_before_the_owner_listens_is_delivered_once_it_does() {
        let (_folder, directory) = folder();
        let owner = primary(&directory);
        let launch = launch(&directory);
        thread::sleep(SETTLE);
        let (activated, activations) = mpsc::channel();

        let _listener = owner.listen(move || activated.send(()).unwrap()).unwrap();

        // The request is already waiting in the pipe, so the owner's read finishes as it starts.
        activations.recv_timeout(PROMPTLY).unwrap();
        assert!(is_secondary(launch.join().unwrap()));
    }

    #[test]
    fn a_launch_takes_the_folder_over_when_the_owner_leaves_without_listening() {
        let (_folder, directory) = folder();
        let owner = primary(&directory);

        drop(owner);

        assert!(matches!(claim(&directory), Ok(Instance::Primary(_))));
    }

    #[test]
    fn a_waiting_launch_takes_the_folder_over_when_the_owner_leaves_without_answering() {
        let (_folder, directory) = folder();
        let owner = primary(&directory);
        let waiting = launch(&directory);
        thread::sleep(SETTLE);

        drop(owner);

        assert!(matches!(waiting.join().unwrap(), Ok(Instance::Primary(_))));
    }

    #[test]
    fn a_request_sent_during_a_handover_reaches_the_next_owner_not_the_leaving_one() {
        let (_folder, directory) = folder();
        let (leaving, leaving_entries, resume_leaving) = held_listener(primary(&directory));
        assert!(is_secondary(claim(&directory)));
        leaving_entries.recv_timeout(WAIT).unwrap();

        // The leaving owner's listener is held inside its activation, so its drop waits there.
        let stopping = thread::spawn(move || drop(leaving));
        let next = primary(&directory);
        let pending = launch(&directory);
        thread::sleep(SETTLE);
        drop(resume_leaving);
        stopping.join().unwrap();
        let (activated, activations) = mpsc::channel();

        let _listener = next.listen(move || activated.send(()).unwrap()).unwrap();

        activations.recv_timeout(WAIT).unwrap();
        assert!(is_secondary(pending.join().unwrap()));
        assert!(
            leaving_entries.try_recv().is_err(),
            "the leaving owner took a request"
        );
    }

    #[test]
    fn a_launch_is_secondary_only_when_its_own_request_was_accepted() {
        let (_folder, directory) = folder();
        let (first, first_entries, resume_first) = held_listener(primary(&directory));
        assert!(is_secondary(claim(&directory)));
        first_entries.recv_timeout(WAIT).unwrap();
        let waiting = launch(&directory);
        thread::sleep(SETTLE);
        let first_stopping = thread::spawn(move || drop(first));
        let Ok(Instance::Primary(next)) = waiting.join().unwrap() else {
            panic!("the waiting launch must take the folder over from the leaving owner");
        };
        let (next, next_entries, resume_next) = held_listener(next);
        assert!(is_secondary(claim(&directory)));
        next_entries.recv_timeout(WAIT).unwrap();

        let last = launch(&directory);
        thread::sleep(SETTLE);
        let next_stopping = thread::spawn(move || drop(next));

        // The next owner is held in the activation of an earlier launch and is leaving: it never
        // accepted the last launch's request, so the last launch must own the folder.
        assert!(matches!(last.join().unwrap(), Ok(Instance::Primary(_))));
        drop(resume_next);
        next_stopping.join().unwrap();
        drop(resume_first);
        first_stopping.join().unwrap();
        assert!(
            next_entries.try_recv().is_err(),
            "an activation without a request"
        );
        assert!(
            first_entries.try_recv().is_err(),
            "an activation without a request"
        );
    }
}
