//! The week's side of « Corriger le temps… »: a declaration of work on the open day, an edit
//! of a correction from the details, and the result the window leaves.

use worktime_domain::time::LocalWorkday;

use super::{Awaited, DayTimeline, Recorded, SegmentKind, WeekState};
use crate::adjust::{Closed, Seed, Stretch, Unrecorded};
use crate::strings::Key;

impl WeekState {
    /// « Déclarer du temps travaillé… » of the open day, as `HistoryWorkspace.DeclareWorkAsync`:
    /// on its gaps still to settle, in the day, when it has some; else on the hour after its
    /// last segment.
    pub fn declare_work(&self) -> Option<(LocalWorkday, Seed)> {
        let workday = self.adjust_target()?;
        let Some(day) = self.timeline.days.iter().find(|day| {
            day.figures
                .as_ref()
                .is_some_and(|figures| figures.workday == workday)
        }) else {
            return Some((workday, Seed::DeclareWork(None)));
        };
        let gaps = day.segments.iter().filter(|segment| {
            segment.needs_action && matches!(segment.kind, SegmentKind::Gap | SegmentKind::Sleep)
        });
        let seed = match Unrecorded::new(
            gaps.map(|segment| segment.interval).collect(),
            stretches(day),
        ) {
            Some(unrecorded) => Seed::DeclareWork(Some(unrecorded)),
            None => day.segments.last().map_or(Seed::DeclareWork(None), |last| {
                Seed::DeclareAfter(last.interval.end())
            }),
        };
        Some((workday, seed))
    }

    /// The correction « Modifier… » named could not be opened: `message` says why.
    pub fn edit_refused(&mut self, message: Key) {
        self.problem = Some(message);
    }

    /// Shows the result of a correction saved in « Corriger le temps… », with « Annuler »
    /// while the week shows a new one; or, when the totals were not refreshed after it, asks
    /// for a refresh as after a command of the week.
    pub fn adjusted(&mut self, closed: Closed) {
        self.awaited = Some(Awaited {
            message: closed.message,
            undoable: closed.undoable,
            undoing: Vec::new(),
        });
        self.finished(Ok(Recorded {
            saved: vec![closed.saved],
            recalculated: closed.refreshed,
            proposal: None,
        }));
    }
}

/// What `day` holds, as « Déclarer du temps travaillé » draws it.
fn stretches(day: &DayTimeline) -> Vec<Stretch> {
    day.segments
        .iter()
        .map(|segment| Stretch {
            kind: segment.kind,
            interval: segment.interval,
        })
        .collect()
}
