//! What the week offers on a manual correction, as the ledger decides it when it reads the week.

use crate::strings::Key;

/// What the week offers on a correction its summaries count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// « Modifier… » and « Retirer la correction ».
    EditAndRemove,
    /// « Modifier… » only: a long-lock exclusion, and every edit of it, stays bound to its
    /// review, which the ledger never withdraws.
    EditOnly,
    /// Neither: a long-lock confirmation, whose replacement would have to confirm a review the
    /// ledger already counts as resolved.
    Locked,
    /// Neither, with nothing to say why: the week could not read its day's history, or the
    /// history does not hold it as a correction to count.
    Unknown,
}

impl Offer {
    /// « Modifier… »
    pub fn edits(self) -> bool {
        matches!(self, Self::EditAndRemove | Self::EditOnly)
    }

    /// « Retirer la correction »
    pub fn removes(self) -> bool {
        self == Self::EditAndRemove
    }

    /// Why the bubble and the details offer less than both: an excluded long lock stays bound to
    /// its review, a confirmed one is locked.
    pub fn why(self) -> Option<Key> {
        match self {
            Self::EditOnly => Some(Key::HistoryBubbleBoundNote),
            Self::Locked => Some(Key::HistoryConfirmationLocked),
            Self::EditAndRemove | Self::Unknown => None,
        }
    }
}
