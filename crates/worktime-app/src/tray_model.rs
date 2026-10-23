//! What the notification-area icon shows and offers, apart from the Windows calls of `tray` so
//! that it is tested everywhere. The menu holds the commands of `_baseTrayCommands` in
//! `App.xaml.cs` and of `DailySummaryTrayCommandBuilder`, in the order the Designer set for R6:
//! an unavailable command stays in the menu, greyed out. Unlike the .NET app, each long lock
//! to review gets a submenu named by its time range, offering the answers of the week view.

use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};
use worktime_domain::accounting::{LongLockReviewId, TrackingState};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;

use crate::live::Tracking;
use crate::strings::{Key, Language};
use crate::workday::TrayIcon;

const ICONS: [(&str, &[u8]); 8] = [
    (
        "indicator-green",
        include_bytes!("../assets/tray/indicator-green.ico"),
    ),
    (
        "indicator-orange",
        include_bytes!("../assets/tray/indicator-orange.ico"),
    ),
    (
        "indicator-red",
        include_bytes!("../assets/tray/indicator-red.ico"),
    ),
    (
        "indicator-blue",
        include_bytes!("../assets/tray/indicator-blue.ico"),
    ),
    (
        "indicator-green-degraded",
        include_bytes!("../assets/tray/indicator-green-degraded.ico"),
    ),
    (
        "indicator-orange-degraded",
        include_bytes!("../assets/tray/indicator-orange-degraded.ico"),
    ),
    (
        "indicator-red-degraded",
        include_bytes!("../assets/tray/indicator-red-degraded.ico"),
    ),
    (
        "indicator-blue-degraded",
        include_bytes!("../assets/tray/indicator-blue-degraded.ico"),
    ),
];

/// The ICO file of `icon`, copied from the tray assets of the first .NET app.
pub fn icon_file(icon: TrayIcon) -> &'static [u8] {
    ICONS
        .iter()
        .find(|(stem, _)| *stem == icon.file_stem())
        .map(|(_, bytes)| *bytes)
        .expect("every tray icon is bundled")
}

/// A long lock that has ended and waits for the user to say what it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedReview {
    pub workday: LocalWorkday,
    pub id: LongLockReviewId,
    pub locked_episode: UtcInterval,
    /// The lock in its day's zone, as the week view shows it: from the start to the end of what
    /// resolving it records, see [`worktime_core::reviews::resolution_interval`], then the
    /// lock's own start, where finishing the day at it ends the day.
    pub local_start: Time,
    pub local_end: Time,
    pub finish_at: Time,
    /// How long the shown range lasts, which its local times alone miss across midnight or a
    /// change of offset.
    pub duration: SignedDuration,
    /// The name the user gave the recurring appointment the absence fell in, empty for none;
    /// `None` for a lock.
    pub appointment: Option<String>,
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
            let label = match &review.appointment {
                Some(name) => language.format(
                    Key::TrayAppointmentReviewSubmenuFormat,
                    &[
                        &appointment_name(name, review.workday.date.weekday(), language),
                        &start,
                        &end,
                    ],
                ),
                // The day ends where the lock began; an appointment is not left for home.
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
