use super::*;

#[test]
fn a_zombie_or_dead_process_has_exited_and_a_running_or_sleeping_one_has_not() {
    assert!(!has_exited("4242 (worktime) S 1 4242 4242 0 -1 4194304"));
    assert!(!has_exited("4242 (worktime) R 1 4242"));
    assert!(!has_exited("4242 (odd) Z name) S 1 4242"));
    assert!(has_exited("4242 (worktime) Z 1 4242"));
    assert!(has_exited("4242 (worktime) X 1 4242"));
    assert!(has_exited("garbled"));
}

#[cfg(target_os = "linux")]
#[test]
fn waits_until_a_child_exits_and_gives_up_on_one_that_runs() {
    waits_until_a_child_exits(wait_for_exit);
}

#[cfg(target_os = "linux")]
#[test]
fn a_pidfd_waits_until_a_child_exits() {
    waits_until_a_child_exits(|pid, timeout| {
        native::wait_on_pidfd(pid, Instant::now() + timeout).expect("Linux 5.3 or later")
    });
}

#[cfg(target_os = "linux")]
#[test]
fn polling_waits_until_a_child_exits() {
    waits_until_a_child_exits(|pid, timeout| poll_stat(pid, Instant::now() + timeout));
}

#[cfg(target_os = "linux")]
fn waits_until_a_child_exits(wait: impl Fn(u32, Duration) -> bool) {
    let mut running = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let started = Instant::now();
    assert!(!wait(running.id(), Duration::from_millis(200)));
    assert!(started.elapsed() >= Duration::from_millis(200));

    let pid = running.id();
    running.kill().unwrap();
    // Not reaped yet: a zombie has exited all the same.
    assert!(wait(pid, Duration::from_secs(5)));
    running.wait().unwrap();
    assert!(wait(pid, Duration::ZERO));
}
