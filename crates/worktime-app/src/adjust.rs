//! The « Corriger le temps… » window as a form: the four corrections of the .NET dialog, their
//! fields, the validation on save and what the window shows while the correction is saved.
//! Ported from `Presentation/Adjustments/AdjustTimeDialogViewModel.cs` and
//! `AdjustTimeSubmission.cs`; nothing here knows GPUI.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{RoundMode, SignedDuration, Timestamp, TimestampRound, Unit};
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::adjustments::{AdjustmentKind, ManualAdjustment, ManualClassification};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;

use crate::strings::{Key, Language};
use crate::time_text::{day_close, duration_text, local_range_text, long_date, minutes_text};
pub use unrecorded::{AroundRow, DayContext, Placed, SlotChoice, Stretch, Unrecorded};

/// The most characters « Motif » takes, as the .NET `MaxLength`.
pub const REASON_MAX_LENGTH: usize = 240;

/// The corrections in the order of the .NET `ComboBox`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Intent {
    #[default]
    AddInterval,
    AddDuration,
    Exclude,
    Reclassify,
}

impl Intent {
    pub const ALL: [Intent; 4] = [
        Intent::AddInterval,
        Intent::AddDuration,
        Intent::Exclude,
        Intent::Reclassify,
    ];

    pub fn label(self, language: Language) -> &'static str {
        language.text(match self {
            Self::AddInterval => Key::AdjustTimeAddIntervalContent,
            Self::AddDuration => Key::AdjustTimeAddDurationContent,
            Self::Exclude => Key::AdjustTimeExcludeContent,
            Self::Reclassify => Key::AdjustTimeReclassifyContent,
        })
    }
}

pub fn classification_label(
    classification: ManualClassification,
    language: Language,
) -> &'static str {
    language.text(match classification {
        ManualClassification::Credited => Key::AdjustTimeClassificationCreditedContent,
        ManualClassification::Excluded => Key::AdjustTimeClassificationExcludedContent,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimePart {
    Hours,
    Minutes,
}

impl TimePart {
    fn max(self) -> i8 {
        match self {
            Self::Hours => 23,
            Self::Minutes => 59,
        }
    }
}

/// A time of day edited as two spin buttons, hours then minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeField {
    hours: i8,
    minutes: i8,
    /// The part whose first digit was just typed: the next digit completes it.
    typing: Option<TimePart>,
}

impl TimeField {
    pub fn new(hours: i8, minutes: i8) -> Self {
        Self {
            hours,
            minutes,
            typing: None,
        }
    }

    pub fn time(&self) -> Time {
        jiff::civil::time(self.hours, self.minutes, 0, 0)
    }

    /// « 09:00 ».
    pub fn text(&self) -> String {
        format!("{:02}:{:02}", self.hours, self.minutes)
    }

    pub fn value(&self, part: TimePart) -> i8 {
        match part {
            TimePart::Hours => self.hours,
            TimePart::Minutes => self.minutes,
        }
    }

    /// « 09 ».
    pub fn part_text(&self, part: TimePart) -> String {
        format!("{:02}", self.value(part))
    }

    fn set(&mut self, part: TimePart, value: i8) {
        match part {
            TimePart::Hours => self.hours = value,
            TimePart::Minutes => self.minutes = value,
        }
    }

    /// ↑ and ↓: one more or one less, round the clock.
    pub fn step(&mut self, part: TimePart, delta: i8) {
        let modulus = part.max() + 1;
        self.set(part, (self.value(part) + delta).rem_euclid(modulus));
        self.typing = None;
    }

    /// A digit typed on `part`. Returns whether the part is complete, so that the focus moves
    /// on: after two digits, or after one that no second digit could follow (« 3 » hours).
    pub fn type_digit(&mut self, part: TimePart, digit: u8) -> bool {
        self.type_digit_up_to(part, digit, part.max())
    }

    /// [`Self::type_digit`], up to `max`: 24 hours for the end of a range, which is 24:00.
    pub fn type_digit_up_to(&mut self, part: TimePart, digit: u8, max: i8) -> bool {
        let digit = digit.min(9) as i8;
        if self.typing == Some(part) {
            let value = self.value(part) * 10 + digit;
            if value <= max {
                self.set(part, value);
                if part == TimePart::Hours && value > TimePart::Hours.max() {
                    self.minutes = 0;
                }
                self.typing = None;
                return true;
            }
        }
        self.set(part, digit);
        if digit * 10 > max {
            self.typing = None;
            true
        } else {
            self.typing = Some(part);
            false
        }
    }

    /// The focus left the field: the next digit starts over.
    pub fn leave(&mut self) {
        self.typing = None;
    }
}

/// What the form opens on.
#[derive(Clone, Debug, PartialEq)]
pub enum Seed {
    /// An interval from 09:00 to 10:00, as the .NET dialog opens.
    Blank,
    /// « Déclarer du temps travaillé », as `DeclareWorkAsync` of the .NET app opens it from
    /// the flyout's source hint and from the week: an interval counted as work over what was
    /// detected, on the unrecorded time shown in its day, if any: every interval the hint
    /// reports, or the slot of the week to count in part.
    DeclareWork(Option<Unrecorded>),
    /// « Déclarer du temps travaillé » of a day with nothing unrecorded, whose time ends at this
    /// instant: on the hour after it from its next whole minute, within the day and up to now.
    DeclareAfter(Timestamp),
    /// « Modifier… » of the week and « Modifier le dernier ajustement… » of the tray, as
    /// `AdjustmentDialogRequest.Edit` of the .NET app: the fields of a correction, saved as its
    /// replacement.
    Edit(Editable),
}

/// A correction the form can replace: one `prefill` opens on.
#[derive(Clone, Debug, PartialEq)]
pub struct Editable(ManualAdjustment);

impl Editable {
    /// `None` for what the form cannot replace.
    pub fn new(adjustment: ManualAdjustment) -> Option<Self> {
        prefill(adjustment.kind()).map(|_| Self(adjustment))
    }

    pub fn workday(&self) -> &LocalWorkday {
        self.0.workday()
    }
}

/// The choices `Prefill` of the .NET dialog makes for `kind`. `None` for a revocation, which
/// only withdraws another, and for a long-lock confirmation: its replacement would have to
/// confirm the same review, which the ledger refuses once the review is resolved, and a plain
/// reclassification would credit the sleep within the lock and could exclude what was confirmed.
fn prefill(kind: &AdjustmentKind) -> Option<Selection> {
    let credited = ManualClassification::Credited;
    let (intent, classification) = match kind {
        AdjustmentKind::Revocation => return None,
        AdjustmentKind::Reclassification { .. } if kind.confirmed_review().is_some() => {
            return None;
        }
        AdjustmentKind::IntervalWork { .. } => (Intent::AddInterval, credited),
        AdjustmentKind::DurationWork { .. } => (Intent::AddDuration, credited),
        AdjustmentKind::Exclusion { .. } => (Intent::Exclude, credited),
        AdjustmentKind::Reclassification { classification, .. } => {
            (Intent::Reclassify, *classification)
        }
    };
    Some(Selection {
        intent,
        classification,
    })
}

/// What a correction records: the user picks it, except on a declaration of work, which always
/// records a credited reclassification.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Selection {
    intent: Intent,
    classification: ManualClassification,
}

/// What the form records, fixed when it opens.
#[derive(Clone, Debug, PartialEq)]
enum Opening {
    /// « Corriger le temps… »: the user picks what to record.
    Correction(Selection),
    /// « Corriger le temps… » on the correction `replaces`, saved as its replacement, which
    /// « Annuler » cannot withdraw: that would not bring back the correction it replaced.
    /// `original` is its interval, whose seconds Début and Fin cannot show: an endpoint left on
    /// its minute keeps them, as `TryEndpoint` of the .NET dialog.
    Edit {
        replaces: AdjustmentId,
        original: Option<UtcInterval>,
        selection: Selection,
    },
    /// « Déclarer du temps travaillé »: Début–Fin counted as work over what was detected,
    /// with no choice, as `TryCreateDraft` of the .NET app. `unrecorded`, when the form opened
    /// on unrecorded time, is that time in its day, and `chosen` the slot of it the fields were
    /// filled from, kept to the second by an endpoint left on its minute as `original` of an
    /// edit.
    Declaration {
        chosen: Option<UtcInterval>,
        unrecorded: Option<Unrecorded>,
    },
}

impl Opening {
    /// `None` for a declaration of work, which has no choice.
    fn selection(&self) -> Option<Selection> {
        match *self {
            Self::Correction(selection) | Self::Edit { selection, .. } => Some(selection),
            Self::Declaration { .. } => None,
        }
    }

    /// `None` for a declaration of work, which has no choice.
    fn selection_mut(&mut self) -> Option<&mut Selection> {
        match self {
            Self::Correction(selection) | Self::Edit { selection, .. } => Some(selection),
            Self::Declaration { .. } => None,
        }
    }
}

/// The day a correction targets, and its zone, in which the fields are local times.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub workday: LocalWorkday,
    pub zone: TimeZone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldError {
    Interval,
    Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Editing,
    Saving,
    /// Saved, but the totals were not recalculated since.
    RefreshPending,
}

/// What a saved correction leaves to the view that opened the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed {
    /// « 09:00–10:00 compté comme travail (1 h 00 min). »
    pub message: String,
    pub saved: AdjustmentId,
    /// The totals were recalculated after the save: the opener shows it at once.
    pub refreshed: bool,
    /// A new correction, which « Annuler » withdraws, rather than the replacement of an edit.
    pub undoable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutTone {
    Error,
    Warning,
}

/// The message pinned above the footer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callout {
    pub tone: CalloutTone,
    pub message: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentOption {
    pub intent: Intent,
    pub label: &'static str,
    pub selected: bool,
}

/// What a declaration of work shows besides the fields of a correction.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclarationText {
    /// Under the day, what a declaration does with the work already detected.
    pub explanation: &'static str,
    /// The unrecorded time the form opened on, in its day.
    pub context: Option<DayContext>,
}

/// What the window shows, every text resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct AdjustPage {
    pub title: &'static str,
    /// « Journée à corriger : jeudi 8 octobre 2026 », or « Journée : … » for a declaration.
    pub day: String,
    pub intent_header: &'static str,
    /// Empty for a declaration of work, which has no choice: the window hides the group.
    pub intents: Vec<IntentOption>,
    /// Début and Fin with « Heures locales · Europe/Paris » under them, the zone of the day
    /// they are times of; `None` for the one Durée field.
    pub local_times: Option<String>,
    pub shows_classification: bool,
    /// Under Début and Fin, which both get the error border.
    pub interval_error: Option<&'static str>,
    pub duration_error: Option<&'static str>,
    pub callout: Option<Callout>,
    pub declaration: Option<DeclarationText>,
    /// Every field and choice is disabled as one unit.
    pub locked: bool,
    /// Enregistrer, or « Actualiser les totaux » once saved.
    pub primary: &'static str,
    pub primary_enabled: bool,
    /// The spinner replaces the primary label.
    pub busy: bool,
    /// Annuler, or Fermer once saved.
    pub secondary: &'static str,
    pub secondary_enabled: bool,
}

/// The form's state; the window changes it through these methods only.
#[derive(Clone, Debug, PartialEq)]
pub struct AdjustForm {
    target: Target,
    language: Language,
    /// The id and time of the correction, the same for every retry, so that saving twice
    /// records it once (`AdjustTimeSubmissionAttempt`).
    id: AdjustmentId,
    created_at: Timestamp,
    opening: Opening,
    pub start: TimeField,
    pub end: TimeField,
    pub duration: String,
    pub reason: String,
    field_error: Option<FieldError>,
    failure: Option<Key>,
    phase: Phase,
    /// The result text of the correction being saved.
    message: Option<String>,
}

impl AdjustForm {
    /// A correction of `target` opened at `now`, on `seed`.
    pub fn new(
        target: Target,
        seed: Seed,
        id: AdjustmentId,
        now: Timestamp,
        language: Language,
    ) -> Self {
        let blank = (TimeField::new(9, 0), TimeField::new(10, 0));
        let mut created_at = now;
        let (mut duration, mut reason) = (String::new(), String::new());
        let (opening, (start, end)) = match seed {
            Seed::Blank => (
                Opening::Correction(Selection {
                    intent: Intent::default(),
                    classification: ManualClassification::Credited,
                }),
                blank,
            ),
            Seed::DeclareWork(unrecorded) => {
                let unrecorded = unrecorded.and_then(|unrecorded| {
                    unrecorded.clamped(|interval| within_day(interval, &target, now))
                });
                let chosen = unrecorded
                    .as_ref()
                    .map(|unrecorded| unrecorded.slots()[unrecorded.first_choice()]);
                (
                    Opening::Declaration { chosen, unrecorded },
                    chosen.map_or(blank, |interval| minute_fields(interval, &target.zone)),
                )
            }
            Seed::DeclareAfter(end) => (
                Opening::Declaration {
                    chosen: None,
                    unrecorded: None,
                },
                hour_after(end, &target, now)
                    .map_or(blank, |interval| minute_fields(interval, &target.zone)),
            ),
            Seed::Edit(Editable(adjustment)) => {
                let selection =
                    prefill(adjustment.kind()).expect("an editable correction has its choices");
                // A replacement must be created after the correction it replaces; the ledger
                // keeps 100 ns ticks.
                created_at = now.max(adjustment.created_at() + SignedDuration::from_nanos(100));
                reason = adjustment.reason().unwrap_or_default().to_owned();
                if let AdjustmentKind::DurationWork { duration: worked } = adjustment.kind() {
                    duration = format!("{}:{:02}", worked.as_hours(), worked.as_mins() % 60);
                }
                let original = adjustment.interval();
                (
                    Opening::Edit {
                        replaces: adjustment.id().clone(),
                        original,
                        selection,
                    },
                    original.map_or(blank, |interval| minute_fields(interval, &target.zone)),
                )
            }
        };
        Self {
            target,
            language,
            id,
            created_at,
            opening,
            start,
            end,
            duration,
            reason,
            field_error: None,
            failure: None,
            phase: Phase::Editing,
            message: None,
        }
    }

    pub fn workday(&self) -> &LocalWorkday {
        &self.target.workday
    }

    /// What Enregistrer records: a declaration of work is the credited reclassification of
    /// `ReclassifyIntervalDraft`, so that work already detected is counted once.
    fn selection(&self) -> Selection {
        self.opening.selection().unwrap_or(Selection {
            intent: Intent::Reclassify,
            classification: ManualClassification::Credited,
        })
    }

    pub fn intent(&self) -> Intent {
        self.selection().intent
    }

    pub fn classification(&self) -> ManualClassification {
        self.selection().classification
    }

    /// Fields and choices take no input while the correction is saved, and after.
    pub fn is_locked(&self) -> bool {
        self.phase != Phase::Editing
    }

    pub fn is_saving(&self) -> bool {
        self.phase == Phase::Saving
    }

    pub fn is_refresh_pending(&self) -> bool {
        self.phase == Phase::RefreshPending
    }

    /// The other fields' error goes with them. A declaration of work has no choice.
    pub fn set_intent(&mut self, intent: Intent) {
        let locked = self.is_locked();
        if let Some(current) = self.opening.selection_mut()
            && !locked
            && intent != current.intent
        {
            current.intent = intent;
            self.field_error = None;
        }
    }

    /// One of the unrecorded slots the declaration opened on: Début and Fin move to it.
    pub fn choose_slot(&mut self, index: usize) {
        if self.is_locked() {
            return;
        }
        let Opening::Declaration {
            chosen,
            unrecorded: Some(unrecorded),
        } = &mut self.opening
        else {
            return;
        };
        let Some(&slot) = unrecorded.slots().get(index) else {
            return;
        };
        *chosen = Some(slot);
        (self.start, self.end) = minute_fields(slot, &self.target.zone);
        self.field_error = None;
    }

    pub fn set_classification(&mut self, classification: ManualClassification) {
        let locked = self.is_locked();
        if let Some(current) = self.opening.selection_mut()
            && !locked
        {
            current.classification = classification;
        }
    }

    /// After a conflict, only closing and reopening helps. A day of a week that has ended stays
    /// so whatever the fields say: saving again can only be refused.
    fn can_save(&self) -> bool {
        self.phase == Phase::Editing
            && !matches!(
                self.failure,
                Some(Key::AdjustTimeErrorConflict | Key::AdjustTimeErrorOutsideWeek)
            )
    }

    /// Enregistrer: the correction to record, or `None` when a field is invalid, which then
    /// shows its error. The form is locked until [`Self::finished`].
    pub fn submit(&mut self) -> Option<ManualAdjustment> {
        if !self.can_save() {
            return None;
        }
        self.field_error = None;
        self.failure = None;
        let Selection {
            intent,
            classification,
        } = self.selection();
        let (adjustment, message) = match intent {
            Intent::AddDuration => {
                let Some(duration) = parse_duration(&self.duration) else {
                    self.field_error = Some(FieldError::Duration);
                    return None;
                };
                (
                    ManualAdjustment::duration_work(
                        self.id.clone(),
                        self.target.workday.clone(),
                        duration,
                        self.created_at,
                    ),
                    self.language.format(
                        Key::AdjustTimeResultDuration,
                        &[&minutes_text(duration, self.language)],
                    ),
                )
            }
            intent => {
                let Some(interval) = self.interval() else {
                    self.field_error = Some(FieldError::Interval);
                    return None;
                };
                let range = self.interval_text(interval);
                let worked = self.language.format(
                    Key::HistoryResultWorked,
                    &[&range, &duration_text(interval.duration(), self.language)],
                );
                let (id, workday, at) = (
                    self.id.clone(),
                    self.target.workday.clone(),
                    self.created_at,
                );
                match (intent, classification) {
                    (Intent::AddInterval, _) => (
                        ManualAdjustment::interval_work(id, workday, interval, at),
                        worked,
                    ),
                    (Intent::Exclude, _) => (
                        ManualAdjustment::exclusion(id, workday, interval, at),
                        self.language.format(Key::HistoryResultNotWorked, &[&range]),
                    ),
                    (_, classification) => (
                        ManualAdjustment::reclassification(
                            id,
                            workday,
                            interval,
                            classification,
                            at,
                        ),
                        match classification {
                            ManualClassification::Credited => worked,
                            ManualClassification::Excluded => self
                                .language
                                .format(Key::HistoryResultReclassified, &[&range]),
                        },
                    ),
                }
            }
        };
        // A target is a working day, the only refusal of these constructors.
        let mut adjustment = adjustment.ok()?.with_reason(&self.reason);
        if let Opening::Edit { replaces, .. } = &self.opening {
            adjustment = adjustment.superseding(replaces.clone());
        }
        self.phase = Phase::Saving;
        self.message = Some(message);
        Some(adjustment)
    }

    /// Début and Fin on the target day. An endpoint still on the minute it opened on keeps that
    /// instant, so an untouched whole day stays whole. A typed time must exist once in the
    /// zone: a skipped or repeated 00:00 is refused like any other time, where [`within_day`]
    /// clamps a suggestion to the day's first instant. Fin typed on 00:00 is the close of the
    /// day, as the week draws it 24:00, unless Début is on 00:00 too: that interval is empty.
    fn interval(&self) -> Option<UtcInterval> {
        let zone = &self.target.zone;
        let day = self.target.workday.date;
        let opened = match self.opening {
            Opening::Edit { original, .. } => original,
            Opening::Declaration { chosen, .. } => chosen,
            Opening::Correction { .. } => None,
        };
        let kept = |field: &TimeField, opened: Option<Timestamp>| {
            opened.filter(|opened| minute_field(*opened, zone).time() == field.time())
        };
        let start = kept(&self.start, opened.map(|interval| interval.start()))
            .or_else(|| local_instant(day.to_datetime(self.start.time()), zone))?;
        let typed_end =
            if self.end.time() == Time::midnight() && self.start.time() != Time::midnight() {
                day_close(day)?
            } else {
                day.to_datetime(self.end.time())
            };
        let end = kept(&self.end, opened.map(|interval| interval.end()))
            .or_else(|| local_instant(typed_end, zone))?;
        UtcInterval::new(start, end).ok()
    }

    /// The host recorded the correction, then recalculated the totals or not; or it refused it
    /// with the message `Err` names. A full success closes the window: its result is returned.
    pub fn finished(&mut self, result: Result<bool, Key>) -> Option<Closed> {
        if self.phase != Phase::Saving {
            return None;
        }
        match result {
            Err(key) => {
                self.phase = Phase::Editing;
                self.failure = Some(key);
                None
            }
            Ok(true) => self.closed(true),
            Ok(false) => {
                self.phase = Phase::RefreshPending;
                None
            }
        }
    }

    /// « Actualiser les totaux » ran a refresh: the window closes once it recalculated.
    pub fn refreshed(&mut self, recalculated: bool) -> Option<Closed> {
        (self.phase == Phase::RefreshPending && recalculated)
            .then(|| self.closed(true))
            .flatten()
    }

    /// What closing the window now leaves to its opener: a saved correction still waiting for
    /// its refresh, or nothing. `None` too while saving, which the window does not allow.
    pub fn close(&mut self) -> Option<Closed> {
        (self.phase == Phase::RefreshPending)
            .then(|| self.closed(false))
            .flatten()
    }

    pub fn can_close(&self) -> bool {
        self.phase != Phase::Saving
    }

    fn closed(&mut self, refreshed: bool) -> Option<Closed> {
        Some(Closed {
            message: self.message.take()?,
            saved: self.id.clone(),
            refreshed,
            undoable: !matches!(self.opening, Opening::Edit { .. }),
        })
    }

    pub fn page(&self) -> AdjustPage {
        let language = self.language;
        let text = |key| language.text(key);
        let pending = self.phase == Phase::RefreshPending;
        let callout = if pending {
            Some(Callout {
                tone: CalloutTone::Warning,
                message: text(Key::AdjustmentSavedRefreshPendingMessage),
            })
        } else {
            self.failure.map(|key| Callout {
                tone: CalloutTone::Error,
                message: text(key),
            })
        };
        let intent = self.intent();
        let (title, day, intents, declaration) = match self.opening {
            Opening::Correction { .. } | Opening::Edit { .. } => (
                text(Key::AdjustTimeDialogTitle),
                Key::AdjustTimeTargetDateFormat,
                Intent::ALL
                    .into_iter()
                    .map(|option| IntentOption {
                        intent: option,
                        label: option.label(language),
                        selected: option == intent,
                    })
                    .collect(),
                None,
            ),
            Opening::Declaration {
                chosen,
                ref unrecorded,
            } => (
                text(Key::DeclareWorkTitle),
                Key::DeclareWorkDateFormat,
                Vec::new(),
                Some(DeclarationText {
                    explanation: text(if unrecorded.is_some() {
                        Key::DeclareWorkContextExplanation
                    } else {
                        Key::DeclareWorkExplanationText
                    }),
                    context: unrecorded.as_ref().map(|unrecorded| {
                        DayContext::new(
                            unrecorded,
                            chosen,
                            self.interval(),
                            &self.target.zone,
                            language,
                        )
                    }),
                }),
            ),
        };
        AdjustPage {
            title,
            day: language.format(day, &[&full_date(self.target.workday.date, language)]),
            intent_header: text(Key::AdjustTimeIntentHeader),
            intents,
            local_times: (intent != Intent::AddDuration).then(|| {
                language.format(
                    Key::AdjustTimeLocalTimes,
                    &[&self.target.workday.time_zone_id],
                )
            }),
            shows_classification: self
                .opening
                .selection()
                .is_some_and(|selection| selection.intent == Intent::Reclassify),
            interval_error: (self.field_error == Some(FieldError::Interval))
                .then(|| text(Key::AdjustTimeErrorInvalidInterval)),
            duration_error: (self.field_error == Some(FieldError::Duration))
                .then(|| text(Key::AdjustTimeErrorInvalidDuration)),
            declaration,
            callout,
            locked: self.is_locked(),
            primary: text(if pending {
                Key::AdjustmentRetryRefreshContent
            } else {
                Key::AdjustTimeDialogPrimaryButtonText
            }),
            primary_enabled: pending || self.can_save(),
            busy: self.phase == Phase::Saving,
            secondary: text(if pending {
                Key::AdjustTimeCloseButton
            } else {
                Key::AdjustTimeDialogCloseButtonText
            }),
            secondary_enabled: self.can_close(),
        }
    }

    /// `interval` in local times as the week writes it: to the second, as the .NET dialog shows
    /// the selected period, on an end that has seconds, and to 24:00 when it closes the day.
    fn interval_text(&self, interval: UtcInterval) -> String {
        local_range_text(interval, &self.target.zone, self.language)
    }
}

/// What of `interval` lies within the target day and not past the minute of `now`, beyond which
/// the accounting refuses a correction. `None` when nothing does.
fn within_day(interval: UtcInterval, target: &Target, now: Timestamp) -> Option<UtcInterval> {
    let horizon = now
        .round(
            TimestampRound::new()
                .smallest(Unit::Minute)
                .mode(RoundMode::Floor),
        )
        .unwrap_or(now);
    let date = target.workday.date;
    UtcInterval::new(
        interval.start().max(midnight(date, &target.zone)?),
        interval
            .end()
            .min(midnight(date.tomorrow().ok()?, &target.zone)?)
            .min(horizon),
    )
    .ok()
}

/// The hour from the first whole minute at or after `end`, so that Début on its minute never
/// overlaps what ends there, cut at the end of the target day and at the minute of `now`.
/// `None` when nothing of it is left.
fn hour_after(end: Timestamp, target: &Target, now: Timestamp) -> Option<UtcInterval> {
    let date = target.workday.date;
    let day = UtcInterval::new(
        midnight(date, &target.zone)?,
        midnight(date.tomorrow().ok()?, &target.zone)?,
    )
    .ok()?;
    let day = within_day(day, target, now)?;
    let start = end
        .round(
            TimestampRound::new()
                .smallest(Unit::Minute)
                .mode(RoundMode::Ceil),
        )
        .ok()?
        .max(day.start());
    UtcInterval::new(
        start,
        (start + SignedDuration::from_hours(1)).min(day.end()),
    )
    .ok()
}

/// The first instant of `date` in `zone`.
fn midnight(date: Date, zone: &TimeZone) -> Option<Timestamp> {
    date.to_zoned(zone.clone()).ok().map(|day| day.timestamp())
}

/// Début and Fin on the minutes of `interval`.
fn minute_fields(interval: UtcInterval, zone: &TimeZone) -> (TimeField, TimeField) {
    (
        minute_field(interval.start(), zone),
        minute_field(interval.end(), zone),
    )
}

/// The minute of `instant` in `zone`, its seconds dropped as the .NET time picker shows them.
fn minute_field(instant: Timestamp, zone: &TimeZone) -> TimeField {
    let local = instant.to_zoned(zone.clone());
    TimeField::new(local.hour(), local.minute())
}

/// `local` in `zone`, unless the time is skipped or repeated there that day.
fn local_instant(local: DateTime, zone: &TimeZone) -> Option<Timestamp> {
    zone.to_ambiguous_timestamp(local).unambiguous().ok()
}

/// « 1:30 »: hours from 0 to 23 and minutes from 0 to 59, more than zero in all.
pub fn parse_duration(text: &str) -> Option<SignedDuration> {
    let (hours, minutes) = text.split_once(':')?;
    let number = |part: &str, max: i64| {
        let part = part.trim();
        (!part.is_empty() && part.chars().all(|digit| digit.is_ascii_digit()))
            .then(|| part.parse::<i64>().ok())
            .flatten()
            .filter(|value| *value <= max)
    };
    let duration = SignedDuration::from_hours(number(hours, 23)?)
        + SignedDuration::from_mins(number(minutes, 59)?);
    (!duration.is_zero()).then_some(duration)
}

/// The long date with its year, as the .NET `D` format writes it: « jeudi 8 octobre 2026 »,
/// "Thursday, October 8, 2026".
pub fn full_date(date: Date, language: Language) -> String {
    let day = long_date(date, language);
    match language {
        Language::French => {
            let mut characters = day.chars();
            let first = characters
                .next()
                .map(|first| first.to_lowercase().collect::<String>());
            format!(
                "{}{} {}",
                first.unwrap_or_default(),
                characters.as_str(),
                date.year()
            )
        }
        Language::English => format!("{day}, {}", date.year()),
    }
}

mod unrecorded;
pub mod window;

#[cfg(test)]
mod declare_tests;
#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;
