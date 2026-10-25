//! Ported from `WorkdayRefreshCoordinatorTests.cs`. The .NET tests that race tasks are reduced to
//! the order in which refreshes begin and complete, which is all a synchronous coordinator sees:
//! `NewerRefreshCancelsAndSuppressesOlderResult`, `PriorCompletionDuringCancellationHandoffIsBenign`
//! and the two tests that alternate the default and a forced refresh all become
//! [`a_newer_refresh_supersedes_an_older_one_that_finishes_later`], since the refresh is passed
//! at every call rather than stored. Left out: `DeliversConcurrentPublicationsInRevisionOrder` and
//! `WaitsForAsyncSubscriberBeforeDeliveringNextPublication`, because the coordinator returns each
//! publication to the caller instead of dispatching it to subscribers. The publications of the
//! weekly recalculation are checked in `weekly/tests.rs`.

use super::*;

#[derive(Debug, PartialEq, Eq)]
struct Failed(&'static str);

fn summary(value: &str) -> Result<String, Failed> {
    Ok(value.to_owned())
}

#[test]
fn refresh_publishes_the_summary_and_advances_the_revision() {
    let mut coordinator = RefreshCoordinator::new();
    let mut published = Vec::new();

    published.push(coordinator.refresh(|_| summary("first")).unwrap().clone());
    published.push(
        coordinator
            .refresh(|previous| summary(&format!("second after {}", previous.unwrap())))
            .unwrap()
            .clone(),
    );

    assert_eq!(
        published,
        [
            Publication {
                revision: 1,
                summary: "first".to_owned()
            },
            Publication {
                revision: 2,
                summary: "second after first".to_owned()
            },
        ]
    );
    assert_eq!(coordinator.current(), Some(&published[1]));
}

#[test]
fn a_newer_refresh_supersedes_an_older_one_that_finishes_later() {
    let mut coordinator = RefreshCoordinator::new();
    let older = coordinator.begin();
    let newer = coordinator.begin();

    assert!(coordinator.is_superseded(&older));
    assert!(!coordinator.is_superseded(&newer));
    assert_eq!(coordinator.complete(older, summary("superseded")), Ok(None));
    assert_eq!(
        coordinator
            .complete(newer, summary("newest"))
            .unwrap()
            .cloned(),
        Some(Publication {
            revision: 1,
            summary: "newest".to_owned()
        })
    );
    assert_eq!(coordinator.current().unwrap().summary, "newest");
}

/// Not in .NET as such: a superseded refresh that fails still reports its failure, and does not
/// release the refresh that superseded it.
#[test]
fn a_superseded_failure_is_reported_and_leaves_the_newer_refresh_in_flight() {
    let mut coordinator = RefreshCoordinator::<String>::new();
    let older = coordinator.begin();
    let newer = coordinator.begin();

    assert_eq!(
        coordinator.complete(older, Err(Failed("source failed"))),
        Err(Failed("source failed"))
    );
    assert!(!coordinator.is_superseded(&newer));
    assert_eq!(
        coordinator
            .complete(newer, summary("newest"))
            .unwrap()
            .map(|publication| publication.revision),
        Some(1)
    );
}

#[test]
fn a_failure_keeps_the_last_published_summary() {
    let mut coordinator = RefreshCoordinator::new();
    coordinator.refresh(|_| summary("healthy")).unwrap();

    assert_eq!(
        coordinator.refresh(|_| Err(Failed("source failed"))),
        Err(Failed("source failed"))
    );
    assert_eq!(
        coordinator.current(),
        Some(&Publication {
            revision: 1,
            summary: "healthy".to_owned()
        })
    );
}

/// `CallerCancellationIsPropagated`: a refresh the caller gives up on publishes nothing, and the
/// next one is published as the first.
#[test]
fn an_abandoned_refresh_publishes_nothing() {
    let mut coordinator = RefreshCoordinator::new();
    drop(coordinator.begin());

    assert_eq!(coordinator.current(), None);
    assert_eq!(
        coordinator.refresh(|_| summary("next")).unwrap().revision,
        1
    );
}
