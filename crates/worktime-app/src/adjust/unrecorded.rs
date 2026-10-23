//! The day around unrecorded time. « Renseigner un créneau travaillé » shows it so that the
//! user sees when the time was and what came before and after it, rather than times to recall.

use jiff::civil::{DateTime, DateTimeRound};
use jiff::tz::TimeZone;
use jiff::{RoundMode, SignedDuration, Timestamp, Unit};
use worktime_domain::intervals::UtcInterval;

use crate::strings::{Key, Language};
use crate::time_text::{
    duration_text, local_range_text, minutes_text, range_ends, range_text, spoken_duration,
};
use crate::week::SegmentKind;

/// One stretch of the day, of a kind the week draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stretch {
    pub kind: SegmentKind,
    pub interval: UtcInterval,
}

/// Unrecorded time to declare, with its day.
#[derive(Clone, Debug, PartialEq)]
pub struct Unrecorded {
    slots: Vec<UtcInterval>,
    day: Vec<Stretch>,
}

impl Unrecorded {
    /// `slots` are the times to declare, `day` what the day held from arrival, both sorted here.
    /// `None` without a slot.
    pub fn new(mut slots: Vec<UtcInterval>, mut day: Vec<Stretch>) -> Option<Self> {
        if slots.is_empty() {
            return None;
        }
        slots.sort_by_key(|slot| slot.start());
        day.sort_by_key(|stretch| stretch.interval.start());
        Some(Self { slots, day })
    }

    pub fn slots(&self) -> &[UtcInterval] {
        &self.slots
    }

    #[cfg(test)]
    pub(crate) fn day(&self) -> &[Stretch] {
        &self.day
    }

    /// Whether the user picks the slot among several.
    pub fn offers_choice(&self) -> bool {
        self.slots.len() > 1
    }

    /// The slot a declaration opens on: the longest, the earliest of equals, as the one most
    /// likely to be work the user remembers.
    pub fn first_choice(&self) -> usize {
        let longest = self
            .slots
            .iter()
            .map(|slot| slot.duration())
            .max()
            .expect("there is a slot");
        self.slots
            .iter()
            .position(|slot| slot.duration() == longest)
            .expect("the longest is a slot")
    }

    /// The same, the slots limited to what `keep` leaves of each; `None` when nothing is left.
    pub(crate) fn clamped(
        &self,
        keep: impl Fn(UtcInterval) -> Option<UtcInterval>,
    ) -> Option<Self> {
        Self::new(
            self.slots.iter().filter_map(|slot| keep(*slot)).collect(),
            self.day.clone(),
        )
    }

    /// What came right before `slot` and right after it, in the day.
    pub fn around(&self, slot: UtcInterval) -> (Option<Stretch>, Option<Stretch>) {
        let before = self
            .day
            .iter()
            .rev()
            .find(|stretch| stretch.interval.end() <= slot.start())
            .copied();
        let after = self
            .day
            .iter()
            .find(|stretch| stretch.interval.start() >= slot.end())
            .copied();
        (before, after)
    }

    /// The kind of the day's stretch under `slot`: a gap when the day has none there.
    pub fn kind_of(&self, slot: UtcInterval) -> SegmentKind {
        self.day
            .iter()
            .find(|stretch| stretch.interval.intersect(slot).is_some())
            .map_or(SegmentKind::Gap, |stretch| stretch.kind)
    }

    /// How the strip draws `stretch`: a gap or a sleep that holds no slot is drawn as a pause,
    /// so that only the time to declare is hatched.
    pub fn drawn_kind(&self, stretch: &Stretch) -> SegmentKind {
        let unrecorded = self
            .slots
            .iter()
            .any(|slot| slot.intersect(stretch.interval).is_some());
        match stretch.kind {
            SegmentKind::Gap | SegmentKind::Sleep if !unrecorded => SegmentKind::Pause,
            kind => kind,
        }
    }

    /// From the start of the day's first stretch, or of the first slot, to the end of the last.
    pub fn span(&self) -> UtcInterval {
        let intervals = self
            .day
            .iter()
            .map(|stretch| stretch.interval)
            .chain(self.slots.iter().copied());
        let (start, end) = intervals.fold(
            (self.slots[0].start(), self.slots[0].end()),
            |(start, end), interval| (start.min(interval.start()), end.max(interval.end())),
        );
        UtcInterval::new(start, end).expect("a slot has a length")
    }
}

/// What the form shows of the unrecorded time it opened on, every text resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct DayContext {
    /// « Votre journée », or « Votre journée · choisissez un créneau » over several slots.
    pub title: &'static str,
    /// « Créneaux non renseignés », the name of the choice of slot.
    pub slots_name: &'static str,
    /// The day's stretches, placed as shares of its span.
    pub strip: Vec<Placed>,
    /// Début–Fin on the strip, while they make an interval that meets the span.
    pub marked: Option<(f32, f32)>,
    /// The span's ends, « 08:50 » and « 12:30 », or « 24:00 » for the midnight closing the day.
    pub from: String,
    pub to: String,
    /// One choice per slot, when there are several.
    pub slots: Vec<SlotChoice>,
    /// What came before the chosen slot, the slot, and what came after.
    pub around: Vec<AroundRow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub kind: SegmentKind,
    pub start: f32,
    pub length: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotChoice {
    /// « 10:42–10:52 »
    pub times: String,
    /// « De 10:42 à 10:52, 9 min 50 s », its accessible name.
    pub name: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AroundRow {
    /// How its swatch is drawn: a gap or a sleep that is not unrecorded time is drawn as a
    /// pause, so that only the slots to declare are hatched.
    pub kind: SegmentKind,
    pub label: &'static str,
    pub times: String,
    pub duration: String,
    /// The chosen slot, between what came before and after.
    pub chosen: bool,
    /// « Travail, de 09:00 à 10:42, 1 h 42 min », its accessible name.
    pub name: String,
}

impl DayContext {
    /// `unrecorded` with `chosen` its slot the fields opened on, and `marked` the interval the
    /// fields make now, in local times of `zone`.
    pub fn new(
        unrecorded: &Unrecorded,
        chosen: Option<UtcInterval>,
        marked: Option<UtcInterval>,
        zone: &TimeZone,
        language: Language,
    ) -> Self {
        let span = unrecorded.span();
        let total = span.duration().as_secs_f64();
        let share = |instant: Timestamp| {
            ((instant.duration_since(span.start()).as_secs_f64() / total).clamp(0., 1.)) as f32
        };
        let place = |interval: UtcInterval| (share(interval.start()), share(interval.end()));
        let [from, to] = minute_bounds(span, zone);
        Self {
            title: language.text(if unrecorded.offers_choice() {
                Key::DeclareWorkDayContextPick
            } else {
                Key::DeclareWorkDayContext
            }),
            slots_name: language.text(Key::DeclareWorkUnrecordedSlots),
            strip: unrecorded
                .day
                .iter()
                .map(|stretch| {
                    let (start, end) = place(stretch.interval);
                    Placed {
                        kind: unrecorded.drawn_kind(stretch),
                        start,
                        length: end - start,
                    }
                })
                .collect(),
            marked: marked
                .and_then(|interval| interval.intersect(span))
                .map(place),
            from,
            to,
            slots: if unrecorded.offers_choice() {
                unrecorded
                    .slots
                    .iter()
                    .map(|slot| SlotChoice::new(*slot, chosen == Some(*slot), zone, language))
                    .collect()
            } else {
                Vec::new()
            },
            around: chosen.map_or_else(Vec::new, |slot| {
                let (before, after) = unrecorded.around(slot);
                let context = |stretch| AroundRow::context(stretch, unrecorded, zone, language);
                before
                    .map(context)
                    .into_iter()
                    .chain([AroundRow::chosen(
                        Stretch {
                            kind: unrecorded.kind_of(slot),
                            interval: slot,
                        },
                        zone,
                        language,
                    )])
                    .chain(after.map(context))
                    .collect()
            }),
        }
    }
}

impl SlotChoice {
    fn new(slot: UtcInterval, selected: bool, zone: &TimeZone, language: Language) -> Self {
        let [start, end] = minute_bounds(slot, zone);
        Self {
            times: minute_range(slot, zone, language),
            name: language.format(
                Key::DeclareWorkSlotName,
                &[&start, &end, &spoken_duration(slot.duration(), language)],
            ),
            selected,
        }
    }
}

impl AroundRow {
    /// The slot the fields opened on, to the second: what a declaration left on it records.
    fn chosen(slot: Stretch, zone: &TimeZone, language: Language) -> Self {
        let label = slot.kind.name(language);
        let [start, end] = minute_bounds(slot.interval, zone);
        Self {
            kind: slot.kind,
            label,
            times: local_range_text(slot.interval, zone, language),
            duration: duration_text(slot.interval.duration(), language),
            chosen: true,
            name: language.format(
                Key::DeclareWorkContextChosenRowName,
                &[
                    &label,
                    &start,
                    &end,
                    &spoken_duration(slot.interval.duration(), language),
                ],
            ),
        }
    }

    /// What came before or after it, to the minute: the context of the slot, drawn as the
    /// strip draws it.
    fn context(
        stretch: Stretch,
        unrecorded: &Unrecorded,
        zone: &TimeZone,
        language: Language,
    ) -> Self {
        let label = stretch.kind.name(language);
        let [start, end] = minute_bounds(stretch.interval, zone);
        // The length of the times the row shows, which a reader adds up.
        let length = minute_length(stretch.interval, zone);
        Self {
            kind: unrecorded.drawn_kind(&stretch),
            label,
            times: minute_range(stretch.interval, zone, language),
            duration: minutes_text(length, language),
            chosen: false,
            name: language.format(
                Key::DeclareWorkContextRowName,
                &[&label, &start, &end, &spoken_duration(length, language)],
            ),
        }
    }
}

/// « 10:42–10:52 », to the minute Début and Fin would show: the context of a slot, not its
/// exact bounds.
fn minute_range(interval: UtcInterval, zone: &TimeZone, language: Language) -> String {
    let [start, end] = minute_ends(interval, zone);
    range_text(start, end, language)
}

/// The two ends `minute_range` writes, as a sentence names them.
fn minute_bounds(interval: UtcInterval, zone: &TimeZone) -> [String; 2] {
    let [start, end] = minute_ends(interval, zone);
    range_ends(start, end)
}

/// The two ends of `interval` in `zone`, each on the minute Début and Fin would show.
fn minute_ends(interval: UtcInterval, zone: &TimeZone) -> [DateTime; 2] {
    [interval.start(), interval.end()].map(|instant| {
        let local = instant.to_zoned(zone.clone()).datetime();
        local
            .round(
                DateTimeRound::new()
                    .smallest(Unit::Minute)
                    .mode(RoundMode::Trunc),
            )
            .unwrap_or(local)
    })
}

/// The length of `interval` from one shown minute to the other: « 09:45–10:00 » is 15 min,
/// although it starts at 09:45:42.
fn minute_length(interval: UtcInterval, zone: &TimeZone) -> SignedDuration {
    // Instants rather than clock times, which a change of offset in the night would skew.
    let [start, end] = minute_ends(interval, zone)
        .map(|local| zone.to_ambiguous_timestamp(local).compatible().ok());
    match (start, end) {
        (Some(start), Some(end)) => end.duration_since(start),
        _ => interval.duration(),
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;

    fn at(time: &str) -> Timestamp {
        format!("2026-10-21T{time}:00Z").parse().unwrap()
    }

    fn interval(start: &str, end: &str) -> UtcInterval {
        UtcInterval::new(at(start), at(end)).unwrap()
    }

    fn stretch(kind: SegmentKind, start: &str, end: &str) -> Stretch {
        Stretch {
            kind,
            interval: interval(start, end),
        }
    }

    fn morning() -> Vec<Stretch> {
        vec![
            stretch(SegmentKind::Work, "08:50", "10:42"),
            stretch(SegmentKind::Gap, "10:42", "10:52"),
            stretch(SegmentKind::Work, "10:52", "11:30"),
            stretch(SegmentKind::Gap, "11:30", "11:33"),
            stretch(SegmentKind::Pause, "11:33", "12:30"),
        ]
    }

    #[test]
    fn opens_on_the_longest_slot() {
        let unrecorded = Unrecorded::new(
            vec![interval("11:30", "11:40"), interval("10:42", "10:45")],
            morning(),
        )
        .unwrap();
        assert_eq!(
            unrecorded.slots()[unrecorded.first_choice()],
            interval("11:30", "11:40")
        );
    }

    #[test]
    fn of_equal_slots_opens_on_the_earliest() {
        let unrecorded = Unrecorded::new(
            vec![interval("11:30", "11:33"), interval("10:42", "10:45")],
            morning(),
        )
        .unwrap();
        assert_eq!(
            unrecorded.slots()[unrecorded.first_choice()],
            interval("10:42", "10:45")
        );
    }

    #[test]
    fn names_what_came_before_and_after_a_slot() {
        let unrecorded = Unrecorded::new(vec![interval("11:30", "11:33")], morning()).unwrap();
        assert_eq!(
            unrecorded.around(interval("11:30", "11:33")),
            (
                Some(stretch(SegmentKind::Work, "10:52", "11:30")),
                Some(stretch(SegmentKind::Pause, "11:33", "12:30")),
            )
        );
        assert_eq!(
            unrecorded.around(interval("08:00", "08:50")),
            (None, Some(stretch(SegmentKind::Work, "08:50", "10:42")))
        );
        assert_eq!(
            unrecorded.kind_of(interval("11:30", "11:33")),
            SegmentKind::Gap
        );
    }

    #[test]
    fn spans_the_day_and_its_slots() {
        let unrecorded = Unrecorded::new(vec![interval("12:30", "12:40")], morning()).unwrap();
        assert_eq!(unrecorded.span(), interval("08:50", "12:40"));
    }

    #[test]
    fn needs_a_slot() {
        assert_eq!(Unrecorded::new(Vec::new(), morning()), None);
        let unrecorded = Unrecorded::new(vec![interval("10:42", "10:52")], morning()).unwrap();
        assert_eq!(unrecorded.clamped(|_| None), None);
    }

    fn context(slots: Vec<UtcInterval>, chosen: &str, language: Language) -> DayContext {
        let unrecorded = Unrecorded::new(slots, morning()).unwrap();
        let chosen = *unrecorded
            .slots()
            .iter()
            .find(|slot| slot.start() == at(chosen))
            .unwrap();
        DayContext::new(
            &unrecorded,
            Some(chosen),
            Some(chosen),
            &TimeZone::UTC,
            language,
        )
    }

    #[test]
    fn shows_the_chosen_slot_between_what_came_before_and_after() {
        let context = context(vec![interval("10:42", "10:52")], "10:42", Language::English);
        assert_eq!(context.title, "Your day");
        assert_eq!(context.slots, [], "one slot is no choice");
        assert_eq!(
            (context.from.as_str(), context.to.as_str()),
            ("08:50", "12:30")
        );
        let rows: Vec<_> = context
            .around
            .iter()
            .map(|row| {
                (
                    row.kind,
                    row.times.as_str(),
                    row.duration.as_str(),
                    row.chosen,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (SegmentKind::Work, "08:50–10:42", "1h\u{a0}52m", false),
                (SegmentKind::Gap, "10:42–10:52", "10m", true),
                (SegmentKind::Work, "10:52–11:30", "38m", false),
            ]
        );
        assert_eq!(
            context.around[0].name,
            "Work, 08:50 to 10:42, 1 hour 52 minutes"
        );
    }

    #[test]
    fn only_the_slots_to_declare_are_hatched() {
        let context = context(vec![interval("10:42", "10:52")], "10:42", Language::French);
        let kinds: Vec<_> = context.strip.iter().map(|piece| piece.kind).collect();
        assert_eq!(
            kinds,
            [
                SegmentKind::Work,
                SegmentKind::Gap,
                SegmentKind::Work,
                SegmentKind::Pause,
                SegmentKind::Pause,
            ],
            "the gap of 11:30 holds no slot"
        );
        let (start, end) = context.marked.unwrap();
        assert!((start - 112. / 220.).abs() < 1e-6 && (end - 122. / 220.).abs() < 1e-6);
    }

    #[test]
    fn several_slots_offer_a_choice_of_the_chosen_one() {
        let context = context(
            vec![interval("10:42", "10:52"), interval("11:30", "11:33")],
            "11:30",
            Language::French,
        );
        assert_eq!(context.title, "Votre journée · choisissez un créneau");
        let slots: Vec<_> = context
            .slots
            .iter()
            .map(|slot| (slot.times.as_str(), slot.selected))
            .collect();
        assert_eq!(slots, [("10:42–10:52", false), ("11:30–11:33", true)]);
        assert_eq!(context.slots[0].name, "De 10:42 à 10:52, 10 minutes");
    }

    #[test]
    fn context_rows_last_from_one_shown_minute_to_the_other() {
        let mut day = morning();
        day[0] = Stretch {
            kind: SegmentKind::Work,
            interval: UtcInterval::new("2026-10-21T08:50:42Z".parse().unwrap(), at("10:42"))
                .unwrap(),
        };
        let unrecorded = Unrecorded::new(vec![interval("10:42", "10:52")], day).unwrap();
        let context = DayContext::new(
            &unrecorded,
            Some(interval("10:42", "10:52")),
            None,
            &TimeZone::UTC,
            Language::English,
        );
        assert_eq!(context.around[0].times, "08:50–10:42");
        assert_eq!(context.around[0].duration, "1h\u{a0}52m");
        assert_eq!(context.marked, None);
    }

    #[test]
    fn a_day_running_to_midnight_ends_at_24_00() {
        let unrecorded = Unrecorded::new(
            vec![UtcInterval::new(at("22:00"), "2026-10-22T00:00:00Z".parse().unwrap()).unwrap()],
            Vec::new(),
        )
        .unwrap();
        let context = DayContext::new(&unrecorded, None, None, &TimeZone::UTC, Language::French);
        assert_eq!(
            (context.from.as_str(), context.to.as_str()),
            ("22:00", "24:00")
        );
    }
}
