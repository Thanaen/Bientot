//! The current week as its window shows it: one timeline per weekday on a common hour range,
//! the slots left to settle, and the texts around them. Ported from
//! `Presentation/History/HistoryTimeline.cs`, `HistoryPresentation.cs`, `HistoryWeekPage.cs`
//! and the state of `HistoryWorkspace.cs`; nothing here knows GPUI.

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::{SignedDuration, Timestamp};
use worktime_domain::accounting::{
    AccountedSegment, AccountingLabel, AccountingResult, AdjustmentId, LongLockReviewId,
};
use worktime_domain::history::{DailySummaryBreakdownEntry, DailySummaryDisposition};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;

use worktime_domain::configuration::AppointmentProposal;

use crate::strings::{Key, Language};
use crate::time_text::{duration_text, long_date, minute_range, minutes_text, range_text};
use crate::tray_model::appointment_name;

pub const DEFAULT_START_HOUR: i8 = 8;
pub const DEFAULT_END_HOUR: i8 = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SegmentKind {
    Work,
    Manual,
    Pause,
    NotWorked,
    Gap,
    Sleep,
    Review,
}

impl SegmentKind {
    pub fn name(self, language: Language) -> &'static str {
        language.text(match self {
            Self::Work => Key::TimelineKindWork,
            Self::Manual => Key::TimelineKindManual,
            Self::Pause => Key::TimelineKindPause,
            Self::NotWorked => Key::TimelineKindNotWorked,
            Self::Gap => Key::TimelineKindGap,
            Self::Sleep => Key::TimelineKindSleep,
            Self::Review => Key::TimelineKindReview,
        })
    }

    /// Filled segments are labelled with their start time, the others with their kind.
    fn is_filled(self) -> bool {
        matches!(self, Self::Work | Self::Manual)
    }

    /// The kind of `segment` of the day `accounting` accounts, as its stored summary will
    /// record it: the same kinds the week draws for that day once it reads it.
    pub(crate) fn of_accounted(segment: &AccountedSegment, accounting: &AccountingResult) -> Self {
        use worktime_domain::accounting::SegmentKind as Accounted;
        let (disposition, reviewed) = match segment.kind() {
            Accounted::Credited => (DailySummaryDisposition::ConfirmedCredit, false),
            Accounted::ProvisionalCredit { review_id } => (
                DailySummaryDisposition::ProvisionalCredit,
                accounting
                    .pending_reviews()
                    .iter()
                    .any(|review| review.id == *review_id),
            ),
            Accounted::Excluded => (DailySummaryDisposition::Excluded, false),
            Accounted::Unknown => (DailySummaryDisposition::Unknown, false),
        };
        Self::classify(
            disposition,
            segment.label(),
            segment.origin().manual_adjustment_id().is_some(),
            reviewed,
        )
    }

    fn of(entry: &DailySummaryBreakdownEntry, has_pending_review: bool) -> Self {
        Self::classify(
            entry.disposition(),
            entry.label(),
            entry.adjustment_id().is_some(),
            entry.review_id().is_some() && has_pending_review,
        )
    }

    /// `adjusted` when a correction made the time, `in_review` when a review still pending
    /// holds it.
    fn classify(
        disposition: DailySummaryDisposition,
        label: AccountingLabel,
        adjusted: bool,
        in_review: bool,
    ) -> Self {
        use DailySummaryDisposition::{Excluded, ProvisionalCredit, Unknown};
        if in_review && disposition == ProvisionalCredit {
            Self::Review
        } else if adjusted {
            if disposition == Excluded {
                Self::NotWorked
            } else {
                Self::Manual
            }
        } else if matches!(
            label,
            AccountingLabel::Suspended | AccountingLabel::OvernightLock
        ) {
            Self::Sleep
        } else if disposition == Unknown {
            Self::Gap
        } else if disposition == Excluded {
            Self::Pause
        } else {
            Self::Work
        }
    }
}

/// Unknown time, a sleep or a night lock that no correction or review covers yet.
pub fn is_correctable(entry: &DailySummaryBreakdownEntry) -> bool {
    entry.interval().is_some()
        && entry.adjustment_id().is_none()
        && entry.review_id().is_none()
        && (entry.disposition() == DailySummaryDisposition::Unknown
            || matches!(
                entry.label(),
                AccountingLabel::Suspended | AccountingLabel::OvernightLock
            ))
}

/// A segment of the week, the same across reloads while its day still shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SegmentId {
    pub date: Date,
    pub start: Timestamp,
}

/// One stretch of a day. `start` and `length` are fractions of the week's hour range, so every
/// day lines up on the same axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub interval: UtcInterval,
    pub start: f32,
    pub length: f32,
    pub local_start: DateTime,
    pub local_end: DateTime,
    pub needs_action: bool,
    pub adjustment_id: Option<AdjustmentId>,
    /// The lock a review segment is a piece of, while the ledger still awaits an answer for it.
    pub review: Option<Lock>,
}

/// A long lock awaiting the user's answer, as the ledger would resolve it now. A day's summary
/// keeps only the pieces of the lock that count provisionally: lunch and other exclusions cut
/// it, and its end may have been excluded while it still goes on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockReview {
    pub id: LongLockReviewId,
    /// From the lock to the unlock, or to now while the lock goes on.
    pub locked_episode: UtcInterval,
    /// What confirming or excluding the lock records, see
    /// [`worktime_core::reviews::resolution_interval`].
    pub resolved: UtcInterval,
    /// The lock goes on: the ledger refuses to resolve it yet.
    pub is_open: bool,
    /// The name of the recurring appointment the absence fell in, empty for none, which the
    /// day cannot finish at the start of; `None` for a lock.
    pub appointment: Option<String>,
    /// The weekday of the lock's workday, which names an appointment left unnamed.
    pub weekday: Weekday,
    /// What counting the lock as work, then excluding it, would change in its day's credit, as
    /// the ledger found by recording each and rolling it back: the sleep within the lock and
    /// the corrections it takes out count there, which the day's pieces don't show.
    pub gains: Option<Gains>,
}

/// What each answer on a lock changes in its day's credit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gains {
    pub count: SignedDuration,
    pub exclude: SignedDuration,
}

/// A [`LockReview`] in its day's zone, as each of its pieces shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lock {
    pub id: LongLockReviewId,
    pub resolved: UtcInterval,
    pub local_start: DateTime,
    pub local_end: DateTime,
    /// Where finishing the day at the start of the lock ends it, unless it is an appointment.
    pub finish_at: Option<Time>,
    /// As [`LockReview::appointment`] and [`LockReview::weekday`].
    pub appointment: Option<String>,
    pub weekday: Weekday,
    pub is_open: bool,
    pub gains: Option<Gains>,
}

impl Segment {
    pub fn id(&self, date: Date) -> SegmentId {
        SegmentId {
            date,
            start: self.interval.start(),
        }
    }

    /// `10:20–11:05`, with seconds only when an end has some.
    pub fn range_text(&self, language: Language) -> String {
        range_text(self.local_start, self.local_end, language)
    }

    pub fn duration(&self) -> SignedDuration {
        self.interval.duration()
    }

    pub fn start_time(&self) -> String {
        self.local_start.time().strftime("%H:%M").to_string()
    }

    /// What the segment counts as, or the appointment a review segment fell in:
    /// « Réunion du jeudi à vérifier ».
    pub fn kind_name(&self, language: Language) -> String {
        match &self.review {
            Some(Lock {
                appointment: Some(name),
                weekday,
                ..
            }) => language.format(
                Key::AppointmentReviewNameFormat,
                &[&appointment_name(name, *weekday, language)],
            ),
            _ => self.kind.name(language).to_owned(),
        }
    }

    /// The segment is a piece of an absence in a recurring appointment.
    pub fn is_appointment(&self) -> bool {
        self.review
            .as_ref()
            .is_some_and(|lock| lock.appointment.is_some())
    }

    /// « Trou, 10:20–11:05 (45 min) », as screen readers read the segment.
    pub fn accessible_name(&self, language: Language) -> String {
        language.format(
            Key::TimelineSegmentNameFormat,
            &[
                &self.kind_name(language),
                &self.range_text(language),
                &duration_text(self.duration(), language),
            ],
        )
    }
}

/// A recurring appointment planned on a day, as the bracket over its strip shows it. `start`
/// and `length` are fractions of the week's hour range, as [`Segment`]'s, cut at the drawn
/// hours and at now.
#[derive(Clone, Debug, PartialEq)]
pub struct AppointmentMarker {
    /// As the settings hold it, empty for the name derived from the weekday.
    pub name: String,
    pub planned: (Time, Time),
    pub start: f32,
    pub length: f32,
    /// The planned start is drawn, with a tick; a start cut by the axis is left open.
    pub opens: bool,
    /// The planned end is drawn, with a tick; an end cut by now or the axis is left open.
    pub closes: bool,
}

impl AppointmentMarker {
    /// « Réunion du jeudi, horaire prévu 14:00–16:00 », for its tooltip and screen readers.
    pub fn accessible_name(&self, date: Date, language: Language) -> String {
        let (start, end) = self.planned;
        language.format(
            Key::TimelineAppointmentNameFormat,
            &[
                &appointment_name(&self.name, date.weekday(), language),
                &range_text(date.to_datetime(start), date.to_datetime(end), language),
            ],
        )
    }
}

/// The narrowest a segment is drawn, so that a short one stays visible.
pub const MINIMUM_SEGMENT_WIDTH: f32 = 3.;

/// Where a span of the strip sits on a strip `width` pixels wide.
pub fn place(start: f32, length: f32, width: f32) -> (f32, f32) {
    if width <= 0. {
        return (0., 0.);
    }
    let placed = (length * width).max(MINIMUM_SEGMENT_WIDTH).min(width);
    ((start * width).clamp(0., width - placed), placed)
}

/// Width of a character of the timeline's labels (IBM Plex Mono at 12 px), with margin.
pub const LABEL_CHARACTER_WIDTH: f32 = 7.2;
/// The segment's inner margin on both sides of its label.
pub const LABEL_PADDING: f32 = 16.;

/// The label of a segment `width` pixels wide on the open day: the start time on a filled
/// segment, the kind's word on the others, and nothing when it does not fit. A five-character
/// time needs 52 px. Never a « ? »: the hatching, the badge and the list already say it.
pub fn segment_label<'a>(
    kind: SegmentKind,
    start_time: &'a str,
    word: &'a str,
    width: f32,
) -> Option<&'a str> {
    let label = if kind.is_filled() { start_time } else { word };
    let needed = label.chars().count() as f32 * LABEL_CHARACTER_WIDTH + LABEL_PADDING;
    (width >= needed).then_some(label)
}

/// The room between two hour labels, besides the labels themselves.
pub const TICK_GAP: f32 = 24.;
const TICK_STEPS: [i8; 5] = [1, 2, 3, 4, 6];

/// The hour axis of a strip `width` pixels wide whose labels are `label_width` wide: each
/// tick's position and its label, every hour or the first step that leaves [`TICK_GAP`]
/// between two labels centred on their ticks. The last hour gets a tick only on the step.
pub fn axis(
    start_hour: i8,
    end_hour: i8,
    width: f32,
    label_width: f32,
    language: Language,
) -> Vec<(f32, String)> {
    let hours = end_hour - start_hour;
    let step = TICK_STEPS
        .into_iter()
        .find(|step| width * f32::from(*step) / f32::from(hours) >= label_width + TICK_GAP)
        .unwrap_or(TICK_STEPS[4]);
    (0..=hours / step)
        .map(|index| {
            let hour = start_hour + index * step;
            (
                f32::from(hour - start_hour) / f32::from(hours),
                language.format(Key::TimelineHourFormat, &[&hour]),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayTone {
    Neutral,
    Reached,
    Short,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentView {
    pub id: SegmentId,
    pub kind: SegmentKind,
    pub start: f32,
    pub length: f32,
    pub start_time: String,
    pub word: &'static str,
    pub accessible_name: String,
    pub role: SegmentRole,
}

/// The bracket over a recurring appointment's planned hours, see [`AppointmentMarker`].
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerView {
    pub start: f32,
    pub length: f32,
    pub opens: bool,
    pub closes: bool,
    pub accessible_name: String,
}

/// What a screen reader meets for a segment, always under its `accessible_name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentRole {
    /// On the open day once summarized, every segment selects its range and opens its bubble.
    Button,
    /// Every other segment is read but does nothing.
    Label,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingItem {
    pub id: SegmentId,
    pub kind: SegmentKind,
    pub range: String,
    pub kind_name: &'static str,
    pub duration: String,
    pub settle_name: String,
}

/// One line of the open day's summary: its label and value in two columns, and the sentence a
/// screen reader reads for both.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryLine {
    pub label: &'static str,
    pub value: String,
    pub tone: DayTone,
    pub accessible_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayFocus {
    /// « À régler (2) », over `pending`; the section is hidden while `pending` is empty.
    pub pending_title: String,
    pub pending: Vec<PendingItem>,
    /// « Écart » when the day has a summary, then « Pauses » and « Non compté ».
    pub summary: Vec<SummaryLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayRow {
    pub date: Date,
    /// « Mar. 1 », the open day too; the long date is in `accessible_name`.
    pub label: String,
    pub accessible_name: String,
    pub status: String,
    /// Today's second line: « Départ prévu 16:54 » or « Objectif atteint ».
    pub detail: Option<String>,
    pub tone: DayTone,
    pub total: String,
    pub target: String,
    pub badge: Option<String>,
    pub focused: bool,
    pub future: bool,
    pub is_today: bool,
    pub now: Option<f32>,
    pub segments: Vec<SegmentView>,
    pub markers: Vec<MarkerView>,
    pub focus: Option<DayFocus>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub title: String,
    pub summary: String,
    pub settle: String,
    pub can_settle: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeekPage {
    pub header: Header,
    pub start_hour: i8,
    pub end_hour: i8,
    pub days: Vec<DayRow>,
}

impl WeekPage {
    pub fn project(timeline: &WeekTimeline, focused: Date, language: Language) -> Self {
        // Days not summarized yet are expected to keep the latest known target.
        let expected_target = timeline
            .days
            .iter()
            .rev()
            .find_map(|day| day.figures.as_ref())
            .map(|latest| latest.nominal_target);
        let days = timeline
            .days
            .iter()
            .map(|day| project_row(day, day.date == focused, expected_target, language))
            .collect();
        Self {
            header: project_header(timeline, expected_target, language),
            start_hour: timeline.start_hour,
            end_hour: timeline.end_hour,
            days,
        }
    }
}

fn project_header(
    timeline: &WeekTimeline,
    expected_target: Option<SignedDuration>,
    language: Language,
) -> Header {
    let monday = timeline.week_start;
    let friday = monday
        .checked_add(jiff::Span::new().days(4))
        .expect("a week has a Friday");
    let title = language.format_dates(
        if monday.month() == friday.month() {
            Key::HistoryWeekTitleSameMonth
        } else {
            Key::HistoryWeekTitle
        },
        &[monday, friday],
    );
    let figures: Vec<_> = timeline
        .days
        .iter()
        .map(|day| day.figures.as_ref())
        .collect();
    let counted = minutes_text(
        figures
            .iter()
            .flatten()
            .map(|figures| figures.credited)
            .sum(),
        language,
    );
    let counting = match expected_target {
        Some(expected) => {
            let target = figures
                .iter()
                .map(|figures| figures.map_or(expected, |day| day.nominal_target))
                .sum();
            language.format(
                Key::HistoryWeekSummary,
                &[&counted, &minutes_text(target, language)],
            )
        }
        None => language.format(Key::HistoryWeekCounted, &[&counted]),
    };
    let pending = timeline.pending_count();
    let summary = match pending {
        0 => counting,
        1 => format!("{counting} · {}", language.text(Key::HistoryPendingOne)),
        _ => format!(
            "{counting} · {}",
            language.format(Key::HistoryPendingMany, &[&pending])
        ),
    };
    Header {
        title,
        summary,
        settle: match pending {
            0 => language.text(Key::HistorySettleNone).to_owned(),
            _ => language.format(Key::HistorySettleAll, &[&pending]),
        },
        can_settle: pending > 0,
    }
}

fn project_row(
    day: &DayTimeline,
    focused: bool,
    expected_target: Option<SignedDuration>,
    language: Language,
) -> DayRow {
    let figures = day.figures.as_ref();
    let (status, detail, tone) = day_status(day, language);
    let pending = day.pending_count();
    let badge = (pending > 0).then(|| language.format(Key::HistoryDayBadge, &[&pending]));
    let accessible_name = [
        Some(long_date(day.date, language)),
        Some(status.clone()),
        detail.clone(),
        figures.map(|figures| {
            language.format(
                Key::HistoryWeekSummary,
                &[
                    &minutes_text(figures.credited, language),
                    &minutes_text(figures.effective_target, language),
                ],
            )
        }),
        badge.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ");
    let selectable = focused && day.selectable();
    let segments = day
        .segments
        .iter()
        .map(|segment| SegmentView {
            id: segment.id(day.date),
            kind: segment.kind,
            start: segment.start,
            length: segment.length,
            start_time: segment.start_time(),
            word: segment.kind.name(language),
            accessible_name: segment.accessible_name(language),
            role: if selectable {
                SegmentRole::Button
            } else {
                SegmentRole::Label
            },
        })
        .collect();
    let markers = day
        .appointments
        .iter()
        .map(|marker| MarkerView {
            start: marker.start,
            length: marker.length,
            opens: marker.opens,
            closes: marker.closes,
            accessible_name: marker.accessible_name(day.date, language),
        })
        .collect();
    let future = day.availability == Availability::Future;
    // A future day keeps the target it is expected to have, so its column lines up.
    let target = figures
        .map(|figures| figures.effective_target)
        .or(expected_target.filter(|_| future));
    DayRow {
        date: day.date,
        label: language.format(
            Key::HistoryDayShortFormat,
            &[
                &language.weekday_abbreviation(day.date.weekday()),
                &day.date.day(),
            ],
        ),
        accessible_name,
        status,
        detail,
        tone,
        total: figures.map_or_else(
            || "—".to_owned(),
            |figures| minutes_text(figures.credited, language),
        ),
        target: target.map_or_else(String::new, |target| {
            language.format(
                Key::HistoryDayTargetFormat,
                &[&minutes_text(target, language)],
            )
        }),
        badge,
        focused,
        future,
        is_today: day.is_today,
        now: day.now,
        segments,
        markers,
        focus: focused.then(|| project_focus(day, language)),
    }
}

/// The day's status, today's second line, and their tone.
fn day_status(day: &DayTimeline, language: Language) -> (String, Option<String>, DayTone) {
    let text = |key| language.text(key).to_owned();
    if day.availability == Availability::Future {
        return (text(Key::HistoryFuture), None, DayTone::Neutral);
    }
    let Some(figures) = &day.figures else {
        return (text(Key::HistoryNoSummary), None, DayTone::Neutral);
    };
    if day.is_today {
        let detail = match figures.projected_departure {
            _ if figures.reached() => Some(text(Key::HistoryDayReached)),
            None => None,
            Some(departure) => Some(language.format(
                Key::HistoryDayTodayDeparture,
                &[&language.short_time(departure)],
            )),
        };
        return (text(Key::HistoryDayToday), detail, DayTone::Neutral);
    }
    if figures.reached() {
        (text(Key::HistoryDayReached), None, DayTone::Reached)
    } else {
        (
            language.format(
                Key::HistoryDayShort,
                &[&minutes_text(
                    figures.effective_target - figures.credited,
                    language,
                )],
            ),
            None,
            DayTone::Short,
        )
    }
}

fn project_focus(day: &DayTimeline, language: Language) -> DayFocus {
    let pending: Vec<_> = day
        .segments
        .iter()
        .filter(|segment| segment.needs_action)
        .map(|segment| {
            let (range, duration) = minute_range(segment.local_start, segment.local_end, language);
            PendingItem {
                id: segment.id(day.date),
                kind: segment.kind,
                range,
                kind_name: segment.kind.name(language),
                duration,
                settle_name: language.format(
                    Key::HistoryPendingSettleName,
                    &[&segment.accessible_name(language)],
                ),
            }
        })
        .collect();
    let difference = day.figures.as_ref().map(|figures| {
        let ahead = figures.reached();
        // From the total as the day shows it, to the minute.
        let on_minute =
            |duration: SignedDuration| duration.round(jiff::Unit::Minute).unwrap_or(duration);
        let gap = minutes_text(
            (on_minute(figures.credited) - on_minute(figures.effective_target)).abs(),
            language,
        );
        SummaryLine {
            label: language.text(Key::HistoryDayDifference),
            value: if ahead {
                format!("+{gap}")
            } else {
                language.format(Key::HistoryNegativeDurationFormat, &[&gap])
            },
            tone: if ahead {
                DayTone::Reached
            } else {
                DayTone::Short
            },
            accessible_name: language.format(
                if ahead {
                    Key::HistoryDayAhead
                } else {
                    Key::HistoryDayBehind
                },
                &[&gap],
            ),
        }
    });
    let total = |label, name, kinds: &[SegmentKind]| {
        let value = minutes_text(day.total(kinds), language);
        SummaryLine {
            label: language.text(label),
            accessible_name: language.format(name, &[&value]),
            value,
            tone: DayTone::Neutral,
        }
    };
    DayFocus {
        pending_title: language.format(Key::HistoryPendingTitle, &[&pending.len()]),
        pending,
        summary: difference
            .into_iter()
            .chain([
                total(
                    Key::HistoryDayPausesLabel,
                    Key::HistoryDayPauses,
                    &[SegmentKind::Pause],
                ),
                total(
                    Key::HistoryDayNotCountedLabel,
                    Key::HistoryDayNotCounted,
                    &[SegmentKind::Gap, SegmentKind::Sleep],
                ),
            ])
            .collect(),
    }
}

/// What the host did with a command it recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The corrections a range saved, which « Annuler » withdraws together.
    pub saved: Vec<AdjustmentId>,
    /// The week was recalculated after the command, so that reading it shows the command.
    pub recalculated: bool,
    /// « Compter chaque jeudi… »: the appointment the correction suggests.
    pub proposal: Option<AppointmentProposal>,
}

/// What the result bar says after an action, and the corrections « Annuler » withdraws.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub message: String,
    pub undo: Vec<AdjustmentId>,
    pub proposal: Option<AppointmentProposal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Success,
    Warning,
    Error,
}

/// The one bar of the week under its header: a problem wins over the last result.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoBar {
    pub message: String,
    pub severity: Severity,
    pub can_undo: bool,
    pub next: Option<String>,
    pub dismissible: bool,
    pub offer: Option<ProposalOffer>,
}

/// « Compter chaque jeudi… » under the result of a correction the same as on the previous
/// days of its weekday.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalOffer {
    /// « Même correction que les deux jeudis précédents. »
    pub note: String,
    /// « Compter chaque jeudi… », which opens the appointment in « Paramètres ».
    pub accept: String,
    /// « Ne plus proposer pour le jeudi ».
    pub dismiss: String,
}

/// The week window's state: the week last read, the open day, its selection, and the last result.
/// The keyboard and the buttons change it through these methods only.
#[derive(Clone, Debug, PartialEq)]
pub struct WeekState {
    pub timeline: WeekTimeline,
    pub language: Language,
    focused: Date,
    selection: Option<Selection>,
    outcome: Option<Outcome>,
    /// Why the last command was refused or failed.
    problem: Option<Key>,
    stale: Option<Stale>,
    /// The command sent to the host, with what its result will say.
    awaited: Option<Awaited>,
    details_open: bool,
}

/// Why the week shown may miss what the ledger holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stale {
    /// The last read failed: the week read before stays shown.
    LoadFailed,
    /// A command was recorded, but the week was not recalculated or not read since. Its slots
    /// may be settled already: none is offered until a refresh shows the week again.
    RefreshPending,
}

#[derive(Clone, Debug, PartialEq)]
struct Awaited {
    message: String,
    /// A range, whose saved corrections « Annuler » withdraws.
    undoable: bool,
    /// The corrections « Annuler » withdraws: if it fails, Ctrl+Z withdraws them again while
    /// the warning shows.
    undoing: Vec<AdjustmentId>,
}

impl WeekState {
    pub fn new(timeline: WeekTimeline, language: Language) -> Self {
        Self {
            focused: timeline.focus(None),
            timeline,
            language,
            selection: None,
            outcome: None,
            problem: None,
            stale: None,
            awaited: None,
            details_open: false,
        }
    }

    pub fn focused(&self) -> Date {
        self.focused
    }

    pub fn page(&self) -> WeekPage {
        WeekPage::project(&self.timeline, self.focused, self.language)
    }

    /// The bubble of the selection; its actions are disabled while a refresh is pending.
    pub fn bubble(&self) -> Option<Bubble> {
        let selection = self.selection?;
        let day = self.timeline.day(selection.date)?;
        let mut bubble = Bubble::project(day, selection, day.is_today, self.language)?;
        if self.stale == Some(Stale::RefreshPending) {
            for action in &mut bubble.actions {
                action.enabled = false;
            }
        }
        Some(bubble)
    }

    /// Shows a newly read week, keeping the open day; a selection whose day shows nothing to
    /// correct any more closes.
    /// `recalculated` when a recalculation of the week preceded the read: the week then shows
    /// every command recorded before.
    pub fn reload(&mut self, timeline: WeekTimeline, recalculated: bool) {
        self.focused = timeline.focus(Some(self.focused));
        self.timeline = timeline;
        if recalculated || self.stale == Some(Stale::LoadFailed) {
            self.stale = None;
        }
        self.renormalize_selection();
        if self.bubble().is_none() {
            self.selection = None;
        }
    }

    /// The week could not be read: the last one stays visible under the message.
    pub fn load_failed(&mut self) {
        if self.stale != Some(Stale::RefreshPending) {
            self.stale = Some(Stale::LoadFailed);
        }
    }

    pub fn select(&mut self, date: Date) {
        if self.timeline.day(date).is_some() && date != self.focused {
            self.focused = date;
            self.selection = None;
        }
    }

    /// The day above (`delta` = -1) or below the open one: ↑ ↓, K and J.
    pub fn move_focus(&mut self, delta: isize) {
        let index = self
            .timeline
            .days
            .iter()
            .position(|day| day.date == self.focused)
            .unwrap_or_default();
        if let Some(day) = index
            .checked_add_signed(delta)
            .and_then(|index| self.timeline.days.get(index))
        {
            let date = day.date;
            self.select(date);
        }
    }

    /// « Régler les trous », « Suivant » and N: opens the first segment left to settle.
    pub fn settle_next(&mut self) {
        let next = self.timeline.pending().next();
        if let Some(id) = next {
            self.open(id);
        }
    }

    /// Closes the bar, except the one asking to refresh: the week stays stale until then.
    pub fn dismiss_result(&mut self) {
        self.outcome = None;
        self.problem = None;
        if self.stale == Some(Stale::LoadFailed) {
            self.stale = None;
        }
    }

    /// The day « Corriger le temps… » corrects: the open one, once summarized, if it is a
    /// working day. None while a refresh is pending, as no slot is offered then either.
    pub fn adjust_target(&self) -> Option<LocalWorkday> {
        if self.stale == Some(Stale::RefreshPending) {
            return None;
        }
        self.timeline
            .day(self.focused)
            .filter(|day| day.availability == Availability::Loaded)
            .and_then(|day| day.figures.as_ref())
            .map(|figures| figures.workday.clone())
            .filter(LocalWorkday::is_working_day)
    }

    /// Ctrl+Z and « Annuler »: withdraws the corrections the last result saved, while the week
    /// still shows each of them as removable.
    pub fn undo(&mut self) -> Option<WeekCommand> {
        if self.stale == Some(Stale::RefreshPending) {
            return None;
        }
        let adjustments = self.outcome.as_ref()?.undo.clone();
        if !self.timeline.shows_removable(&adjustments) {
            return None;
        }
        let workday = self.timeline.workday_of(adjustments.first()?)?;
        // Sent once: a second press would find the week changed.
        if let Some(outcome) = &mut self.outcome {
            outcome.undo.clear();
        }
        self.awaited = Some(Awaited {
            message: self.language.text(Key::HistoryResultUndone).to_owned(),
            undoable: false,
            undoing: adjustments.clone(),
        });
        Some(WeekCommand::Revoke {
            workday,
            adjustments,
        })
    }

    /// Records what the host did with the command last chosen, once the week was reloaded
    /// after it. A command recorded without the week shown again asks for a refresh: its
    /// result and « Annuler » show once a refresh succeeds.
    pub fn finished(&mut self, result: Result<Recorded, Key>) {
        let Some(awaited) = self.awaited.take() else {
            return;
        };
        match result {
            Err(key) => {
                self.problem = Some(key);
                if let Some(outcome) = &mut self.outcome {
                    outcome.undo.extend(awaited.undoing);
                }
            }
            Ok(recorded) => {
                self.problem = None;
                if !recorded.recalculated || self.stale.is_some() {
                    self.stale = Some(Stale::RefreshPending);
                }
                self.outcome = Some(Outcome {
                    message: awaited.message,
                    undo: if awaited.undoable {
                        recorded.saved
                    } else {
                        Vec::new()
                    },
                    proposal: recorded.proposal,
                });
            }
        }
    }

    pub fn info_bar(&self) -> Option<InfoBar> {
        let problem = match self.stale {
            Some(Stale::RefreshPending) => {
                Some((Key::HistorySavedRefreshPending, Severity::Warning))
            }
            Some(Stale::LoadFailed) => Some((Key::HistoryLoadFailed, Severity::Error)),
            None => self.problem.map(|key| (key, Severity::Warning)),
        };
        if let Some((key, severity)) = problem {
            return Some(InfoBar {
                message: self.language.text(key).to_owned(),
                severity,
                can_undo: false,
                next: None,
                dismissible: self.stale != Some(Stale::RefreshPending),
                offer: None,
            });
        }
        let outcome = self.outcome.as_ref()?;
        let pending = self.timeline.pending_count();
        Some(InfoBar {
            message: outcome.message.clone(),
            severity: Severity::Success,
            can_undo: self.timeline.shows_removable(&outcome.undo),
            next: (pending > 0).then(|| {
                self.language
                    .format(Key::HistoryResultNextFormat, &[&pending])
            }),
            dismissible: true,
            offer: outcome
                .proposal
                .as_ref()
                .map(|proposal| offer(proposal, self.language)),
        })
    }

    /// The appointment the last correction suggests, until the user answered it.
    pub fn proposal(&self) -> Option<&AppointmentProposal> {
        self.outcome.as_ref()?.proposal.as_ref()
    }

    /// The user opened or turned down the proposal: the result bar no longer offers it.
    pub fn answer_proposal(&mut self) {
        if let Some(outcome) = &mut self.outcome {
            outcome.proposal = None;
        }
    }
}

fn offer(proposal: &AppointmentProposal, language: Language) -> ProposalOffer {
    let (day, days) = (
        language.weekday_name(proposal.weekday),
        language.weekday_plural(proposal.weekday),
    );
    let before = proposal.occurrences.len().saturating_sub(1);
    let count = language.text(Key::HistoryProposalCounts);
    let count = count
        .split(';')
        .nth(before.saturating_sub(2))
        .unwrap_or_default();
    ProposalOffer {
        note: language.format(
            if proposal.alternate_weeks_from.is_some() {
                Key::HistoryProposalAlternateNoteFormat
            } else {
                Key::HistoryProposalNoteFormat
            },
            &[&count, &days],
        ),
        accept: language.format(
            if proposal.alternate_weeks_from.is_some() {
                Key::HistoryProposalAlternateFormat
            } else {
                Key::HistoryProposalEveryWeekFormat
            },
            &[&day],
        ),
        dismiss: language.format(Key::HistoryProposalDismissFormat, &[&days]),
    }
}

mod adjust;
mod bubble;
mod details;
mod offer;
mod selection;
mod timeline;

pub use bubble::{Answer, Bubble, BubbleAction, Held, WeekCommand};
pub use details::{CorrectionRow, CountedCorrection, Credit, Details, DetailsAction};
pub use offer::Offer;
pub use selection::{Edge, RANGE_STEP, Selection};
pub use timeline::{Availability, DayTimeline, Figures, WeekTimeline};

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
