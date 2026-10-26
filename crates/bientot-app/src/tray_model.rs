//! What the notification-area icon shows and offers, apart from the Windows calls of `tray` so
//! that it is tested everywhere. The menu holds the commands of `_baseTrayCommands` in
//! `App.xaml.cs` and of `DailySummaryTrayCommandBuilder`, in the order the Designer set for R6:
//! an unavailable command stays in the menu, greyed out. Unlike the .NET app, each long lock
//! to review gets a submenu named by its time range, offering the answers of the week view.

use bientot_domain::accounting::{
    LongLockReview, LongLockReviewId, LongLockReviewReason, TrackingState,
};
use bientot_domain::configuration::RecurringAppointment;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::LocalWorkday;
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};

use crate::live::Tracking;
use crate::strings::{Key, Language};

/// A long lock that has ended and waits for the user to say what it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedReview {
    pub workday: LocalWorkday,
    pub id: LongLockReviewId,
    pub locked_episode: UtcInterval,
    /// The lock in its day's zone, as the week view shows it: from the start to the end of what
    /// resolving it records, see [`bientot_core::reviews::resolution_interval`], then the
    /// lock's own start, where finishing the day at it ends the day.
    pub local_start: Time,
    pub local_end: Time,
    pub finish_at: Time,
    /// How long the shown range lasts, which its local times alone miss across midnight or a
    /// change of offset.
    pub duration: SignedDuration,
    pub subject: ReviewSubject,
}

/// What a review asks about, which names it and says whether the day can finish at it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewSubject {
    /// A long lock: the day can finish where it began.
    Lock,
    /// An absence in a recurring appointment, by the name the user gave it, empty for none.
    /// An appointment is not left for home.
    Appointment(String),
    /// The session open before Bientôt watched, read from the Windows logs.
    RebuiltSession,
}

impl ReviewSubject {
    /// The subject of `review`, an appointment named among `appointments`, from which it may
    /// be gone since.
    pub fn of(review: &LongLockReview, appointments: &[RecurringAppointment]) -> Self {
        match review.reason {
            LongLockReviewReason::ThresholdExceeded => Self::Lock,
            LongLockReviewReason::RecurringAppointment => {
                Self::Appointment(review.appointment_name(appointments).unwrap_or_default())
            }
            LongLockReviewReason::RebuiltSession => Self::RebuiltSession,
        }
    }

    /// The day can finish at the start of a lock, and of nothing else.
    pub fn is_lock(&self) -> bool {
        *self == Self::Lock
    }

    /// « Rendez-vous du jeudi », « Avant Bientôt »: what the review is named after, on a
    /// workday of `weekday`; `None` for a lock, which its time range names.
    pub fn name(&self, weekday: Weekday, language: Language) -> Option<String> {
        match self {
            Self::Lock => None,
            Self::Appointment(name) => Some(appointment_name(name, weekday, language)),
            Self::RebuiltSession => Some(language.text(Key::RebuiltReviewName).to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewAction {
    ConfirmWork,
    Exclude,
    FinishAtLockStart,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewCommand {
    pub review: ClosedReview,
    pub action: ReviewAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    Open,
    /// Pauses a running day, resumes a paused or finished one.
    TogglePause,
    Finish,
    /// « Corriger le temps… » of the working day shown.
    CorrectTime,
    Review(ReviewCommand),
    /// Opens the latest adjustment of the day in « Corriger le temps… », to replace it.
    EditLatestAdjustment,
    ExportDiagnostics,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        label: String,
        command: MenuCommand,
        enabled: bool,
    },
    Submenu {
        label: String,
        items: Vec<MenuEntry>,
    },
    Separator,
}

/// What the menu depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuState {
    /// Unlike the .NET app, whose tray keeps them, the menu greys the pause toggle and
    /// « Terminer » on a day off, as its flyout does.
    pub tracking: Tracking,
    pub reviews: Vec<ClosedReview>,
    /// The published working day has an adjustment the user could edit; greyed on a day off
    /// by the calendar, as `EditTimeAsync` of the .NET app does nothing then.
    pub has_latest_adjustment: bool,
}

/// `name`, the one the user gave an appointment on `weekday`, or « Rendez-vous du jeudi »
/// without one.
pub fn appointment_name(name: &str, weekday: Weekday, language: Language) -> String {
    let name = name.trim();
    if name.is_empty() {
        language.format(
            Key::AppointmentDefaultNameFormat,
            &[&language.weekday_name(weekday)],
        )
    } else {
        name.to_owned()
    }
}

/// The menu, its first item the default one.
pub fn menu(state: &MenuState, language: Language) -> Vec<MenuEntry> {
    let item = |label: String, command, enabled| MenuEntry::Item {
        label,
        command,
        enabled,
    };
    let text = |key| language.text(key).to_owned();

    let time = |time| language.short_time(time);
    let mut closed: Vec<&ClosedReview> = state.reviews.iter().collect();
    closed.sort_by_key(|review| review.locked_episode.start());
    let mut reviews: Vec<MenuEntry> = closed
        .into_iter()
        .map(|review| {
            let answer = |label, action| {
                item(
                    label,
                    MenuCommand::Review(ReviewCommand {
                        review: review.clone(),
                        action,
                    }),
                    true,
                )
            };
            let (start, end) = (time(review.local_start), time(review.local_end));
            let mut items = vec![
                answer(
                    text(Key::HistoryBubbleConfirmContent),
                    ReviewAction::ConfirmWork,
                ),
                answer(
                    text(Key::HistoryBubbleReclassifyContent),
                    ReviewAction::Exclude,
                ),
            ];
            let label = match review.subject.name(review.workday.date.weekday(), language) {
                Some(name) => language.format(
                    Key::TrayAppointmentReviewSubmenuFormat,
                    &[&name, &start, &end],
                ),
                // The day ends where the lock began.
                None => {
                    items.push(answer(
                        language.format(Key::HistoryBubbleFinish, &[&time(review.finish_at)]),
                        ReviewAction::FinishAtLockStart,
                    ));
                    language.format(Key::TrayReviewSubmenuFormat, &[&start, &end])
                }
            };
            MenuEntry::Submenu { label, items }
        })
        .collect();
    if state.has_latest_adjustment {
        reviews.push(item(
            text(Key::TrayEditLatestAdjustmentCommand),
            MenuCommand::EditLatestAdjustment,
            state.tracking.can_correct_time(),
        ));
    }

    let running = state.tracking.state() == TrackingState::Running;
    let mut entries = vec![
        item(text(Key::TrayOpenCommand), MenuCommand::Open, true),
        MenuEntry::Separator,
        item(
            text(if running {
                Key::TrayPauseCommand
            } else {
                Key::TrayResumeCommand
            }),
            MenuCommand::TogglePause,
            state.tracking.can_toggle(),
        ),
        item(
            text(Key::TrayFinishCommand),
            MenuCommand::Finish,
            state.tracking.can_finish(),
        ),
        item(
            text(Key::FlyoutAdjustTime),
            MenuCommand::CorrectTime,
            state.tracking.can_correct_time(),
        ),
    ];
    if !reviews.is_empty() {
        entries.push(MenuEntry::Separator);
        entries.append(&mut reviews);
        entries.push(MenuEntry::Separator);
    }
    entries.extend([
        item(
            text(Key::TrayExportDiagnosticsCommand),
            MenuCommand::ExportDiagnostics,
            true,
        ),
        MenuEntry::Separator,
        item(text(Key::TrayQuitCommand), MenuCommand::Quit, true),
    ]);
    entries
}

/// The id of the item at `path` in the menu of `generation`: `3:4.1` names the second item of
/// the submenu of the fifth entry. A new menu gets a new generation, so that a click on an item
/// of a menu since replaced names no item.
pub fn item_id(generation: u64, path: &[usize]) -> String {
    let path: Vec<String> = path.iter().map(usize::to_string).collect();
    format!("{generation}:{}", path.join("."))
}

/// What the item `id` does, when it names an enabled item of `entries`, the menu of
/// `generation`.
pub fn chosen(entries: &[MenuEntry], generation: u64, id: &str) -> Option<MenuCommand> {
    let (shown, path) = id.split_once(':')?;
    if shown != generation.to_string() {
        return None;
    }
    let mut indices = path.split('.').map(|index| index.parse::<usize>().ok());
    let mut entries = entries;
    loop {
        match entries.get(indices.next()??)? {
            MenuEntry::Submenu { items, .. } => entries = items,
            MenuEntry::Item {
                command,
                enabled: true,
                ..
            } => return indices.next().is_none().then(|| command.clone()),
            MenuEntry::Item { .. } | MenuEntry::Separator => return None,
        }
    }
}

#[cfg(test)]
mod tests;
