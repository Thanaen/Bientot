//! Which refresh gets published, ported from `Services/WorkdayRefreshCoordinator.cs`.
//!
//! .NET runs refreshes concurrently: a newer refresh cancels the one in flight, and only results
//! that no later refresh superseded are published, numbered in order. Here the host owns the
//! threads. It calls [`RefreshCoordinator::begin`] where a refresh is asked for, runs the
//! refresh wherever it likes, and hands the outcome to [`RefreshCoordinator::complete`] on the
//! thread that owns the coordinator. A refresh that finished after a later one began publishes
//! nothing, and [`RefreshCoordinator::is_superseded`] lets it stop early.

/// A summary as published, numbered from 1 in publication order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication<S> {
    pub revision: u64,
    pub summary: S,
}

/// A refresh that began. Completing it consumes it, so each refresh completes at most once.
#[must_use = "a refresh that never completes publishes nothing"]
#[derive(Debug, PartialEq, Eq)]
pub struct RefreshTicket(u64);

/// Holds the latest publication and the one refresh allowed to replace it.
#[derive(Debug)]
pub struct RefreshCoordinator<S> {
    issued: u64,
    active: Option<u64>,
    current: Option<Publication<S>>,
}

impl<S> Default for RefreshCoordinator<S> {
    fn default() -> Self {
        Self {
            issued: 0,
            active: None,
            current: None,
        }
    }
}

impl<S> RefreshCoordinator<S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&Publication<S>> {
        self.current.as_ref()
    }

    /// Starts a refresh, superseding the one in flight.
    pub fn begin(&mut self) -> RefreshTicket {
        self.issued += 1;
        self.active = Some(self.issued);
        RefreshTicket(self.issued)
    }

    pub fn is_superseded(&self, ticket: &RefreshTicket) -> bool {
        self.active != Some(ticket.0)
    }

    /// Publishes the summary a refresh produced, unless a later refresh began meanwhile. A
    /// failure is returned whether or not the refresh was superseded, and keeps the last
    /// publication.
    pub fn complete<E>(
        &mut self,
        ticket: RefreshTicket,
        outcome: Result<S, E>,
    ) -> Result<Option<&Publication<S>>, E> {
        let superseded = self.is_superseded(&ticket);
        if !superseded {
            self.active = None;
        }
        let summary = outcome?;
        if superseded {
            return Ok(None);
        }
        let revision = self
            .current
            .as_ref()
            .map_or(1, |publication| publication.revision + 1);
        Ok(Some(self.current.insert(Publication { revision, summary })))
    }

    /// Runs `refresh` on this thread, with the summary published last, and publishes its result.
    pub fn refresh<E>(
        &mut self,
        refresh: impl FnOnce(Option<&S>) -> Result<S, E>,
    ) -> Result<&Publication<S>, E> {
        let ticket = self.begin();
        let outcome = refresh(
            self.current
                .as_ref()
                .map(|publication| &publication.summary),
        );
        self.complete(ticket, outcome)
            .map(|published| published.expect("nothing else began while the refresh ran"))
    }
}

#[cfg(test)]
mod tests;
