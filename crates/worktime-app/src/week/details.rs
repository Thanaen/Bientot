//! « Détails de l’activité et du crédit »: the credit of the open day and the corrections its
//! summary counts, as `HistoryFormatter.ProjectDetail`, `RenderDetail` and `OnDetailsClick` of
//! the .NET week show the selected day and bring it into view.

use std::collections::HashMap;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Zoned};
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::history::{DailySummaryDisposition, DailySummarySnapshot};
use worktime_domain::time::LocalWorkday;

use super::{DayTone, Figures, Offer, WeekState};
use crate::adjust::Seed;
use crate::strings::{Key, Language};
use crate::time_text::{duration_text, long_date, range_text};

/// The carry of a day's summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credit {
    pub earned: SignedDuration,
    pub applied: SignedDuration,
    /// Available credit minus debt: negative while the week owes time.
    pub balance: SignedDuration,
    pub expired: SignedDuration,
}

impl Credit {
    pub(super) fn of(snapshot: &DailySummarySnapshot) -> Self {
        Self {
            earned: snapshot.carry_earned(),
            applied: snapshot.carry_applied(),
            balance: snapshot.net_carry_balance(),
            expired: snapshot.carry_expired(),
        }
    }

    /// One of its values: « 2 h 00 min », or « −1 h 30 min » for debt.
    pub fn value_text(value: SignedDuration, language: Language) -> String {
        if value.is_negative() {
            language.format(
                Key::HistoryNegativeDurationFormat,
                &[&duration_text(value.abs(), language)],
            )
        } else {
            duration_text(value, language)
        }
    }
}

/// A correction a day's summary counts: its pieces of the breakdown, gathered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountedCorrection {
    pub id: AdjustmentId,
    pub offer: Offer,
    pub disposition: DailySummaryDisposition,
    /// Its intervals in the day's zone, touching ones merged; none for a duration.
    pub ranges: Vec<(Zoned, Zoned)>,
    pub duration: SignedDuration,
}

/// The corrections of `snapshot` in the order its breakdown first meets them. A withdrawn or
/// replaced correction is no longer counted, so it is not listed.
pub(super) fn corrections(
    snapshot: &DailySummarySnapshot,
    offers: &HashMap<AdjustmentId, Offer>,
    zone: &TimeZone,
) -> Vec<CountedCorrection> {
    let mut gathered: Vec<CountedCorrection> = Vec::new();
    for entry in snapshot.breakdown().entries() {
        let Some(id) = entry.adjustment_id() else {
            continue;
        };
        let index = gathered
            .iter()
            .position(|seen| seen.id == *id)
            .unwrap_or_else(|| {
                gathered.push(CountedCorrection {
                    id: id.clone(),
                    offer: offers.get(id).copied().unwrap_or(Offer::Unknown),
                    disposition: entry.disposition(),
                    ranges: Vec::new(),
                    duration: SignedDuration::ZERO,
                });
                gathered.len() - 1
            });
        let correction = &mut gathered[index];
        correction.duration += entry.duration();
        // The breakdown orders its timed entries by interval, so a piece can only touch the
        // last range gathered.
        if let Some(interval) = entry.interval() {
            let start = interval.start().to_zoned(zone.clone());
            let end = interval.end().to_zoned(zone.clone());
            match correction.ranges.last_mut() {
                Some((_, last)) if *last >= start => {
                    if end > *last {
                        *last = end;
                    }
                }
                _ => correction.ranges.push((start, end)),
            }
        }
    }
    gathered
}

/// « Modifier » of a listed correction, or « Ajouter une correction ».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailsAction {
    pub label: &'static str,
    pub enabled: bool,
}

/// One line of the credit: « Gagné » and « 2 h 00 min ».
#[derive(Clone, Debug, PartialEq)]
pub struct CreditLine {
    pub label: &'static str,
    pub value: String,
    /// [`DayTone::Short`] on debt.
    pub tone: DayTone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CorrectionRow {
    pub id: AdjustmentId,
    pub disposition: &'static str,
    pub ranges: Vec<String>,
    pub duration: String,
    pub edit: DetailsAction,
    /// [`Offer::why`], shown under its row.
    pub note: Option<&'static str>,
    /// « Modifier, Travail confirmé, 15:15–16:00, 45 min », then its note.
    pub edit_name: String,
}

/// The panel under the toggle of the open day, while it is open on a day with a summary.
#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    /// The name a screen reader gives the panel: its title, which the panel does not show,
    /// followed by the open day, as in "Activity and credit, Thursday, September 3".
    pub accessible_name: String,
    pub credit_title: &'static str,
    pub credit: [CreditLine; 4],
    pub corrections_title: &'static str,
    /// Empty when the day counts no correction.
    pub corrections: Vec<CorrectionRow>,
    pub add: DetailsAction,
}

impl WeekState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    /// Whether the open day has a summary to detail: the toggle shows only then.
    pub fn has_details(&self) -> bool {
        self.focused_figures().is_some()
    }

    /// « Détails de l’activité et du crédit »: shows or hides the panel, on every day.
    pub fn toggle_details(&mut self) -> bool {
        self.details_open = !self.details_open;
        self.details_open
    }

    fn focused_figures(&self) -> Option<&Figures> {
        self.timeline.day(self.focused)?.figures.as_ref()
    }

    /// Whether « Modifier » can open « Corriger le temps… » on the correction `id` of the open
    /// day: while the day can be corrected, and never on a long-lock confirmation, as in its
    /// bubble.
    fn correction_editable(&self, id: &AdjustmentId) -> bool {
        self.adjust_target().is_some()
            && self
                .timeline
                .day(self.focused)
                .and_then(|day| day.offer_of(id))
                .is_some_and(Offer::edits)
    }

    /// The panel of the open day, while it is open and the day has a summary.
    pub fn details(&self) -> Option<Details> {
        if !self.details_open {
            return None;
        }
        let language = self.language;
        let figures = self.focused_figures()?;
        let text = |key| language.text(key);
        let duration = |duration| duration_text(duration, language);
        let credit = figures.credit;
        let line = |key, value: SignedDuration| CreditLine {
            label: text(key),
            value: Credit::value_text(value, language),
            tone: if value.is_negative() {
                DayTone::Short
            } else {
                DayTone::Neutral
            },
        };
        let balance = if credit.balance.is_negative() {
            Key::HistoryDebtLabelText
        } else {
            Key::HistoryAvailableLabelText
        };
        let edit = text(Key::HistoryEditContent);
        let corrections = figures
            .corrections
            .iter()
            .map(|correction| {
                let disposition = text(disposition_key(correction.disposition));
                let ranges: Vec<_> = correction
                    .ranges
                    .iter()
                    .map(|(start, end)| range_text(start.datetime(), end.datetime(), language))
                    .collect();
                let duration = duration(correction.duration);
                let note = correction.offer.why().map(text);
                let edit_name = [edit, disposition]
                    .into_iter()
                    .chain(ranges.iter().map(String::as_str))
                    .chain([duration.as_str()])
                    .chain(note)
                    .collect::<Vec<_>>()
                    .join(", ");
                CorrectionRow {
                    id: correction.id.clone(),
                    disposition,
                    ranges,
                    duration,
                    edit: DetailsAction {
                        label: edit,
                        enabled: self.correction_editable(&correction.id),
                    },
                    note,
                    edit_name,
                }
            })
            .collect();
        Some(Details {
            accessible_name: format!(
                "{}, {}",
                text(Key::HistoryDetailsTitleText),
                long_date(self.focused, language)
            ),
            credit_title: text(Key::HistoryCreditTitleText),
            credit: [
                line(Key::HistoryEarnedLabelText, credit.earned),
                line(Key::HistoryAppliedLabelText, credit.applied),
                line(balance, credit.balance),
                line(Key::HistoryExpiredLabelText, credit.expired),
            ],
            corrections_title: text(Key::HistoryManualTitleText),
            corrections,
            add: DetailsAction {
                label: text(Key::HistoryAddContent),
                enabled: self.adjust_target().is_some(),
            },
        })
    }

    /// « Modifier » of a listed correction: the day to open « Corriger le temps… » on, as
    /// « Modifier… » of its bubble.
    pub fn edit_listed(&self, id: &AdjustmentId) -> Option<LocalWorkday> {
        self.adjust_target()
            .filter(|_| self.correction_editable(id))
    }

    /// « Ajouter une correction »: « Corriger le temps… » on the open day.
    pub fn add_listed(&self) -> Option<(LocalWorkday, Seed)> {
        Some((self.adjust_target()?, Seed::Blank))
    }
}

fn disposition_key(disposition: DailySummaryDisposition) -> Key {
    match disposition {
        DailySummaryDisposition::ConfirmedCredit => Key::HistoryDispositionConfirmedCredit,
        DailySummaryDisposition::ProvisionalCredit => Key::HistoryDispositionProvisionalCredit,
        DailySummaryDisposition::Excluded => Key::HistoryDispositionExcluded,
        DailySummaryDisposition::Unknown => Key::HistoryDispositionUnknown,
        DailySummaryDisposition::ManualCredit => Key::HistoryDispositionManualCredit,
    }
}

#[cfg(test)]
#[path = "details_tests.rs"]
mod tests;
