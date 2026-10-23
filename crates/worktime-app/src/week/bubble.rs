//! The bubble under the selected range: what the range holds, « Compter comme travail » and
//! « Exclure du travail » in that order, and the command of the ledger each answer records.

use jiff::SignedDuration;
use jiff::civil::Time;
use worktime_domain::accounting::{AdjustmentId, LongLockReviewId};
use worktime_domain::adjustments::ManualClassification;
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;

use super::{Awaited, DayTimeline, Lock, Offer, Segment, SegmentKind, Selection, WeekState};
use crate::strings::{Key, Language};
use crate::time_text::{minutes_text, range_text};

/// What the user picks in the bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The range counts as work.
    Count,
    /// The range is excluded from work.
    Exclude,
    /// « Retirer la correction » on the correction clicked.
    Remove,
    /// Finishes today at the start of the lock clicked.
    Finish,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BubbleAction {
    pub answer: Answer,
    pub label: String,
    pub enabled: bool,
    /// « Déjà compté comme travail »: the range already counts so, in the action's place.
    pub already: bool,
    /// The likely answer, drawn with the accent.
    pub accent: bool,
    /// Enter answers it: the accent one, on a range that counts one way throughout.
    pub enter: bool,
}

/// How a segment counts now, as both actions see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Counted {
    Credited,
    Excluded,
    /// Left to settle: a gap, a sleep, or a lock to review that counts provisionally.
    Unsettled,
}

impl Counted {
    fn of(kind: SegmentKind) -> Self {
        match kind {
            SegmentKind::Work | SegmentKind::Manual => Self::Credited,
            SegmentKind::Pause | SegmentKind::NotWorked => Self::Excluded,
            SegmentKind::Gap | SegmentKind::Sleep | SegmentKind::Review => Self::Unsettled,
        }
    }
}

impl Answer {
    fn classification(self) -> Option<ManualClassification> {
        match self {
            Self::Count => Some(ManualClassification::Credited),
            Self::Exclude => Some(ManualClassification::Excluded),
            Self::Remove | Self::Finish => None,
        }
    }

    /// The segments that already count as this answer asks.
    fn keeps(self, counted: Counted) -> bool {
        matches!(
            (self, counted),
            (Self::Count, Counted::Credited) | (Self::Exclude, Counted::Excluded)
        )
    }

    /// How much `kind` changes the day's credit once answered so.
    fn credit_change(self, kind: SegmentKind) -> i32 {
        match (self, kind) {
            (
                Self::Count,
                SegmentKind::Pause | SegmentKind::NotWorked | SegmentKind::Gap | SegmentKind::Sleep,
            ) => 1,
            (Self::Exclude, SegmentKind::Work | SegmentKind::Manual | SegmentKind::Review) => -1,
            _ => 0,
        }
    }
}

impl DayTimeline {
    /// How much answering `range` so changes the day's credit. Each long lock the range
    /// settles changes it as the ledger found when it read the week; elsewhere, each piece
    /// counts or stops counting. Sleep within a lock confirmed as work stays uncounted, as the
    /// ledger leaves it.
    fn change(&self, range: UtcInterval, answer: Answer) -> SignedDuration {
        let mut locks: Vec<&Lock> = self
            .segments
            .iter()
            .filter_map(|segment| segment.review.as_ref())
            .filter(|lock| lock.resolved.overlaps(range))
            .collect();
        locks.sort_by_key(|lock| lock.resolved.start());
        locks.dedup_by(|left, right| left.id == right.id);
        let settled: SignedDuration = locks
            .iter()
            .filter_map(|lock| lock.gains)
            .map(|gains| match answer {
                Answer::Count => gains.count,
                Answer::Exclude => gains.exclude,
                Answer::Remove | Answer::Finish => SignedDuration::ZERO,
            })
            .sum();
        let measured: Vec<UtcInterval> = locks
            .iter()
            .filter(|lock| lock.gains.is_some())
            .map(|lock| lock.resolved)
            .collect();
        // A lock the ledger could not answer on trial: its sleep at least stays uncounted.
        let unmeasured: Vec<UtcInterval> = locks
            .iter()
            .filter(|lock| lock.gains.is_none())
            .map(|lock| lock.resolved)
            .collect();
        let pieces: SignedDuration = self
            .pieces(range)
            .flat_map(|(segment, part)| {
                outside(part, &measured)
                    .into_iter()
                    .map(move |part| (segment, part))
            })
            .map(|(segment, part)| {
                let asleep_in_locks = match (answer, segment.kind) {
                    (Answer::Count, SegmentKind::Sleep) if segment.adjustment_id.is_some() => {
                        part.duration()
                    }
                    (Answer::Count, SegmentKind::Sleep) => unmeasured
                        .iter()
                        .filter_map(|lock| lock.intersect(part))
                        .map(|within| within.duration())
                        .sum(),
                    _ => SignedDuration::ZERO,
                };
                (part.duration() - asleep_in_locks) * answer.credit_change(segment.kind)
            })
            .sum();
        settled + pieces
    }
}

/// What of `interval` lies outside every one of `cuts`.
fn outside(interval: UtcInterval, cuts: &[UtcInterval]) -> Vec<UtcInterval> {
    cuts.iter().fold(vec![interval], |pieces, cut| {
        pieces
            .into_iter()
            .flat_map(|piece| {
                if !piece.overlaps(*cut) {
                    return vec![piece];
                }
                [
                    UtcInterval::new(piece.start(), cut.start()).ok(),
                    UtcInterval::new(cut.end(), piece.end()).ok(),
                ]
                .into_iter()
                .flatten()
                .collect()
            })
            .collect()
    })
}

/// A kind of time a range holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub kind: SegmentKind,
    /// « Trou », as the legend names it.
    pub name: String,
    /// « 15 min », when the range holds more than one kind.
    pub duration: Option<String>,
}

/// The bubble of the selection; at most one action has the accent.
#[derive(Clone, Debug, PartialEq)]
pub struct Bubble {
    pub selection: Selection,
    /// « 11:50–14:00 ».
    pub title: String,
    /// « 2 h 10 min ».
    pub duration: String,
    /// What the range holds: its kind when it holds one, else each kind with its duration.
    pub breakdown: Vec<Held>,
    /// De and À, in the day's zone.
    pub from: Time,
    pub to: Time,
    /// « Ou faites glisser les bords. », beside them while the range has handles.
    pub hint: Option<&'static str>,
    /// The segment clicked takes no answer: no handles, De and À only read.
    pub fixed: bool,
    /// Where the range starts on the hour range, and how much of it it takes.
    pub start: f32,
    pub length: f32,
    pub actions: Vec<BubbleAction>,
    /// What else an answer does, or why one is not offered.
    pub notes: Vec<String>,
    /// « Total du jour si vous excluez : 7 h 38 min → 6 h 08 min. », for the accent action.
    pub preview: Option<String>,
}

impl Bubble {
    /// The bubble offers `answer`, enabled.
    pub fn offers(&self, answer: Answer) -> bool {
        self.actions
            .iter()
            .any(|action| action.answer == answer && action.enabled)
    }

    /// What Enter answers, if anything.
    pub fn entered(&self) -> Option<Answer> {
        self.actions
            .iter()
            .find(|action| action.enter && action.enabled)
            .map(|action| action.answer)
    }

    pub(super) fn project(
        day: &DayTimeline,
        selection: Selection,
        is_today: bool,
        language: Language,
    ) -> Option<Self> {
        let figures = day.figures.as_ref()?;
        let range = selection.range;
        let pieces: Vec<(&Segment, UtcInterval)> = day.pieces(range).collect();
        if pieces.is_empty() {
            return None;
        }
        let anchor = selection.anchor.and_then(|id| {
            day.segments
                .iter()
                .find(|segment| segment.id(day.date) == id)
        });
        let correction = anchor.filter(|segment| segment.adjustment_id.is_some());
        let classes: Vec<Counted> = pieces
            .iter()
            .map(|(segment, _)| Counted::of(segment.kind))
            .collect();
        let uniform = classes.windows(2).all(|pair| pair[0] == pair[1]);
        let change = |answer: Answer| day.change(range, answer);
        let likely = if classes.contains(&Counted::Unsettled) {
            Answer::Count
        } else if classes.iter().all(|counted| Answer::Count.keeps(*counted)) {
            Answer::Exclude
        } else if classes
            .iter()
            .all(|counted| Answer::Exclude.keeps(*counted))
        {
            Answer::Count
        } else if change(Answer::Exclude).abs() > change(Answer::Count) {
            Answer::Exclude
        } else {
            Answer::Count
        };

        // A correction bound to its review counts as its review settled it; one whose history
        // the week could not read is left alone.
        // Sleep a confirmed lock left uncounted: counting the range passes over it.
        let asleep_in_confirmed = |segment: &Segment| {
            segment.kind == SegmentKind::Sleep && segment.adjustment_id.is_some()
        };
        let bound = |answer: Answer| {
            pieces.iter().find_map(|(segment, _)| {
                if asleep_in_confirmed(segment) {
                    // A lock that slept all along leaves the week no piece to name its offer.
                    return (answer != Answer::Count)
                        .then(|| day.offer(segment).unwrap_or(Offer::Locked));
                }
                let offer = day.offer(segment)?;
                let replaced = !answer.keeps(Counted::of(segment.kind));
                (replaced && !offer.removes()).then_some(offer)
            })
        };
        let open_lock = pieces
            .iter()
            .any(|(segment, _)| segment.review.as_ref().is_some_and(|lock| lock.is_open));
        let mut notes = Vec::new();
        if open_lock {
            notes.push(language.text(Key::HistoryRangeOpenLockNote).to_owned());
        }
        let mut actions = Vec::new();
        for answer in [Answer::Count, Answer::Exclude] {
            let already = classes.iter().all(|counted| answer.keeps(*counted));
            if already && correction.is_some() {
                // The breakdown already names the correction's way.
                continue;
            }
            let passed_over = answer == Answer::Count
                && pieces
                    .iter()
                    .all(|(segment, _)| asleep_in_confirmed(segment));
            let enabled = !already && !open_lock && !passed_over && bound(answer).is_none();
            let accent = answer == likely && enabled;
            actions.push(BubbleAction {
                answer,
                label: language
                    .text(match (answer, already) {
                        (Answer::Count, false) => Key::HistoryRangeCountContent,
                        (Answer::Count, true) => Key::HistoryRangeAlreadyCounted,
                        (_, false) => Key::HistoryRangeExcludeContent,
                        (_, true) => Key::HistoryRangeAlreadyExcluded,
                    })
                    .to_owned(),
                enabled,
                already,
                accent,
                enter: accent && uniform,
            });
        }
        if let Some(segment) = correction {
            actions.push(BubbleAction {
                answer: Answer::Remove,
                label: language.text(Key::HistoryBubbleRemoveContent).to_owned(),
                enabled: day.can_remove(segment),
                already: false,
                accent: false,
                enter: false,
            });
        }
        if let Some(lock) = anchor.and_then(|segment| segment.review.as_ref())
            && let Some(finish_at) = lock.finish_at
            && is_today
            && !lock.is_open
        {
            actions.push(BubbleAction {
                answer: Answer::Finish,
                label: language.format(
                    Key::HistoryBubbleFinish,
                    &[&finish_at.strftime("%H:%M").to_string()],
                ),
                enabled: true,
                already: false,
                accent: false,
                enter: false,
            });
        }

        let offer = [Answer::Count, Answer::Exclude].into_iter().find_map(bound);
        // One paragraph with where to change it instead, when it can be.
        notes.extend(offer.and_then(Offer::why).map(|key| match offer {
            Some(Offer::EditOnly) => format!(
                "{} {}",
                language.text(key),
                language.text(Key::HistoryRangeEditInDetails)
            ),
            _ => language.text(key).to_owned(),
        }));
        // The notes say why a range that takes no answer has nothing to drag.
        let fixed = anchor.is_some() && !actions.iter().any(|action| action.enabled);
        let hint = (!fixed).then(|| language.text(Key::HistoryRangeDragHint));
        if correction.is_some() && !fixed {
            notes.push(language.text(Key::HistoryRangeCorrectionNote).to_owned());
        }
        let has_lock = classes.len() > 1
            && pieces
                .iter()
                .any(|(segment, _)| segment.kind == SegmentKind::Review);
        if has_lock && !open_lock {
            notes.push(language.text(Key::HistoryRangeLockNote).to_owned());
        }
        if correction.is_none() {
            let replaced: Vec<&Segment> = pieces
                .iter()
                .map(|(segment, _)| *segment)
                .filter(|segment| {
                    // A lock is settled whole, so a correction within it goes either way.
                    let in_lock = pieces.iter().any(|(other, _)| {
                        other.review.as_ref().is_some_and(|lock| {
                            lock.resolved.overlaps(segment.interval) && segment.review.is_none()
                        })
                    });
                    segment.adjustment_id.is_some()
                        && (in_lock || !likely.keeps(Counted::of(segment.kind)))
                        && !(likely == Answer::Count && asleep_in_confirmed(segment))
                })
                .collect();
            match replaced.as_slice() {
                [] => {}
                [one] => notes.push(language.format(
                    Key::HistoryRangeReplacesOne,
                    &[&clock(one.local_start), &clock(one.local_end)],
                )),
                many => notes.push(language.format(Key::HistoryRangeReplacesMany, &[&many.len()])),
            }
        }

        let preview = actions
            .iter()
            .find(|action| action.accent)
            .map(|action| action.answer)
            .map(|answer| {
                if change(answer) == SignedDuration::ZERO {
                    return language
                        .text(match answer {
                            Answer::Count => Key::HistoryRangeCountNoChange,
                            _ => Key::HistoryRangeExcludeNoChange,
                        })
                        .to_owned();
                }
                language.format(
                    match answer {
                        Answer::Count => Key::HistoryRangeCountPreview,
                        _ => Key::HistoryRangeExcludePreview,
                    },
                    // To the minute before adding, so that the change reads as the durations shown.
                    &[
                        &minutes_text(minute(figures.credited), language),
                        &minutes_text(minute(figures.credited) + minute(change(answer)), language),
                    ],
                )
            });

        // To the minute, as De and À show them.
        let on_minute = |local: jiff::civil::DateTime| local.round(jiff::Unit::Minute);
        let (from, to) = (
            on_minute(day.local(range.start())?).ok()?,
            on_minute(day.local(range.end())?).ok()?,
        );
        let (start, end) = (day.fraction(range.start())?, day.fraction(range.end())?);
        Some(Self {
            selection,
            title: range_text(from, to, language),
            duration: minutes_text(to.duration_since(from), language),
            breakdown: breakdown(&pieces, language),
            from: from.time(),
            to: to.time(),
            hint,
            fixed,
            start,
            length: end - start,
            actions,
            notes,
            preview,
        })
    }
}

fn minute(duration: SignedDuration) -> SignedDuration {
    duration.round(jiff::Unit::Minute).unwrap_or(duration)
}

fn clock(local: jiff::civil::DateTime) -> String {
    local.time().strftime("%H:%M").to_string()
}

/// The name of `kind` in the bubble: what it counts as, as the user would say it.
fn breakdown_name(kind: SegmentKind) -> Key {
    match kind {
        SegmentKind::Work => Key::TimelineKindWork,
        SegmentKind::Manual => Key::TimelineKindManual,
        SegmentKind::Pause => Key::TimelineKindPause,
        SegmentKind::NotWorked => Key::TimelineKindNotWorked,
        SegmentKind::Gap => Key::TimelineKindGap,
        SegmentKind::Sleep => Key::TimelineKindSleep,
        SegmentKind::Review => Key::HistoryBubbleReviewTitle,
    }
}

/// One line per kind the range holds, and per appointment, in the order it meets them: its
/// name alone when there is one line, else with its time to the minute.
fn breakdown(pieces: &[(&Segment, UtcInterval)], language: Language) -> Vec<Held> {
    // An appointment is named apart from the locks and from the other appointments.
    let name = |segment: &Segment| {
        if segment.is_appointment() {
            segment.kind_name(language)
        } else {
            language.text(breakdown_name(segment.kind)).to_owned()
        }
    };
    let mut kinds: Vec<(SegmentKind, String, SignedDuration)> = Vec::new();
    for (segment, part) in pieces {
        let named = name(segment);
        match kinds
            .iter_mut()
            .find(|(kind, held, _)| *kind == segment.kind && *held == named)
        {
            Some((_, _, duration)) => *duration += part.duration(),
            None => kinds.push((segment.kind, named, part.duration())),
        }
    }
    let alone = kinds.len() == 1;
    kinds
        .into_iter()
        .map(|(kind, name, duration)| Held {
            kind,
            name,
            duration: (!alone).then(|| minutes_text(duration, language)),
        })
        .collect()
}

/// A correction of the week the host records on the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeekCommand {
    /// Counts `range` as work or excludes it, see [`worktime_core::ranges`].
    Range {
        workday: LocalWorkday,
        range: UtcInterval,
        classification: ManualClassification,
    },
    FinishAtLockStart {
        workday: LocalWorkday,
        review: LongLockReviewId,
    },
    /// Withdraws `adjustments`, all at once.
    Revoke {
        workday: LocalWorkday,
        adjustments: Vec<AdjustmentId>,
    },
}

impl WeekCommand {
    /// The day whose time the command counts as work, after which WorkTime may propose an
    /// appointment.
    pub fn counted_workday(&self) -> Option<&LocalWorkday> {
        match self {
            Self::Range {
                workday,
                classification: ManualClassification::Credited,
                ..
            } => Some(workday),
            _ => None,
        }
    }
}

impl WeekState {
    /// The command behind `answer` in the open bubble, which closes.
    pub fn choose(&mut self, answer: Answer) -> Option<WeekCommand> {
        let language = self.language;
        let bubble = self.bubble()?;
        if !bubble.offers(answer) {
            return None;
        }
        let selection = bubble.selection;
        let day = self.timeline.day(selection.date)?;
        let workday = day.figures.as_ref()?.workday.clone();
        let anchor = selection
            .anchor
            .and_then(|id| self.timeline.segment(id))
            .map(|(_, segment)| segment);
        let (command, message) = match answer {
            Answer::Count | Answer::Exclude => {
                let counted = matches!(answer, Answer::Count);
                let gained = day.change(selection.range, answer);
                (
                    WeekCommand::Range {
                        workday,
                        range: selection.range,
                        classification: answer.classification()?,
                    },
                    if !counted && gained < SignedDuration::ZERO {
                        language.format(Key::HistoryResultReclassified, &[&bubble.title])
                    } else if !counted {
                        language.format(Key::HistoryResultNotWorked, &[&bubble.title])
                    } else if gained > SignedDuration::ZERO
                        && anchor.is_none_or(|segment| segment.review.is_none())
                    {
                        language.format(
                            Key::HistoryResultWorked,
                            &[&bubble.title, &minutes_text(gained, language)],
                        )
                    } else {
                        language.format(Key::HistoryResultConfirmed, &[&bubble.title])
                    },
                )
            }
            Answer::Finish => {
                let lock = anchor?.review.as_ref()?;
                (
                    WeekCommand::FinishAtLockStart {
                        workday,
                        review: lock.id.clone(),
                    },
                    language.format(
                        Key::HistoryResultFinished,
                        &[&lock.finish_at?.strftime("%H:%M").to_string()],
                    ),
                )
            }
            Answer::Remove => (
                WeekCommand::Revoke {
                    workday,
                    adjustments: vec![anchor?.adjustment_id.clone()?],
                },
                language.text(Key::HistoryResultRemoved).to_owned(),
            ),
        };
        self.selection = None;
        self.awaited = Some(Awaited {
            message,
            undoable: matches!(answer, Answer::Count | Answer::Exclude),
            undoing: Vec::new(),
        });
        Some(command)
    }
}
