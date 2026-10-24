//! Which window « Corriger le temps… » shows when a view asks for it while one may be open, and
//! the view that then shows the result of a saved correction.

use super::{AdjustForm, Opening};

/// What became of a request to open « Corriger le temps… ».
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenOutcome {
    /// The window shows the form asked for: a new one in place of any open one left unchanged,
    /// or the open one, unchanged and already on it.
    Fresh,
    /// The open window came to the front as it is: it was changed, or there was nothing to
    /// open in its place.
    Kept,
    /// No window shows: there was no day to correct, the correction could not be read, or the
    /// window failed to open.
    Refused,
}

/// The view that opened the window, which shows the result of a saved correction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opener {
    Week,
    Flyout,
    Tray,
}

/// The window open on « Corriger le temps… »: its form as it opened, with what its text fields
/// showed then, and as it shows now.
#[derive(Clone, Copy, Debug)]
pub struct OpenWindow<'a> {
    pub opened: &'a AdjustForm,
    pub shown: &'a AdjustForm,
}

/// What a request for « Corriger le temps… » does with the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    /// Open a window on the form asked for, in place of the open one, left unchanged.
    Open,
    /// Bring the open window to the front: unchanged, it already shows the form asked for, and
    /// shows the result to the view that asked.
    Show,
    /// Bring the open window to the front as it is: it was changed, or there is nothing to
    /// open in its place.
    Keep,
    /// No window shows: there is no day to correct.
    Refuse,
}

impl WindowRequest {
    /// The request for `asked`, `None` without a day to correct, with `window` open or none.
    pub fn of(window: Option<OpenWindow<'_>>, asked: Option<&AdjustForm>) -> Self {
        match (window, asked) {
            (None, Some(_)) => Self::Open,
            (None, None) => Self::Refuse,
            (Some(window), _) if window.shown != window.opened => Self::Keep,
            (Some(_), None) => Self::Keep,
            (Some(window), Some(asked)) if window.opened.same_request(asked) => Self::Show,
            (Some(_), Some(_)) => Self::Open,
        }
    }

    /// The view the open window shows a saved correction to: `asking`, which asked for what the
    /// window shows, else still `current`.
    pub fn opener(self, current: Opener, asking: Opener) -> Opener {
        if self == Self::Show { asking } else { current }
    }

    /// What became of the request once the open window was dealt with, `None` while a window
    /// is still to open in its place.
    pub fn outcome(self) -> Option<OpenOutcome> {
        match self {
            Self::Show => Some(OpenOutcome::Fresh),
            Self::Keep => Some(OpenOutcome::Kept),
            Self::Open | Self::Refuse => None,
        }
    }
}

impl AdjustForm {
    /// The height of its window, in logical pixels, fixed when it opens: a form asked for with
    /// another opening gets a window of its own. Each fits its tallest state in French, with
    /// three lines of « Correction enregistrée… » above the footer: for a correction, the four
    /// types, Début and Fin and the classification; for a declaration, which has neither types
    /// nor classification, three lines of explanation and the day of the unrecorded time.
    pub fn window_height(&self) -> f32 {
        match self.opening {
            Opening::Correction { .. } | Opening::Edit { .. } => 680.,
            Opening::Declaration {
                unrecorded: Some(ref unrecorded),
                ..
            } => {
                // A choice of slot takes one more row.
                if unrecorded.offers_choice() {
                    620.
                } else {
                    580.
                }
            }
            // Its three lines of explanation, Début and Fin and Motif, then the result: no
            // more, or the window shows a blank under Motif while the form is filled.
            Opening::Declaration {
                unrecorded: None, ..
            } => 500.,
        }
    }

    /// Whether this form and `other` make the same request: the same correction of the same day,
    /// the fields filled alike, and nothing typed, saved or refused. They differ only by the id
    /// and the creation time each request gives its form. Destructured whole, so that a field
    /// added to the form has to be sorted on one side or the other.
    fn same_request(&self, other: &Self) -> bool {
        let Self {
            target,
            language,
            id: _,
            created_at: _,
            opening,
            start,
            end,
            duration,
            reason,
            field_error,
            failure,
            phase,
            message,
        } = self;
        *target == other.target
            && *language == other.language
            && *opening == other.opening
            && *start == other.start
            && *end == other.end
            && *duration == other.duration
            && *reason == other.reason
            && *field_error == other.field_error
            && *failure == other.failure
            && *phase == other.phase
            && *message == other.message
    }
}
