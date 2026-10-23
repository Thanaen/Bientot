//! A launch on Saturday 10 October 2026, a day off no demo scenario falls on:
//! `flyout-day-off.png` and `week-day-off.png` after a worked week, and `week-no-activity.png`
//! for a fresh install, whose week has no recorded activity.

use gpui::{AppContext as _, HeadlessAppContext, size};
use worktime_app::demo;
use worktime_app::live::Tracking;
use worktime_app::strings::Language;
use worktime_app::theme::size as token;
use worktime_app::week::WeekTimeline;
use worktime_app::week_view::WeekView;
use worktime_app::workday::DayView;
use worktime_domain::accounting::TrackingState;

use super::capture;
use super::fixtures::{Behind, on_saturday};
use super::shared::open_flyout;

/// Captures three files: `flyout-day-off.png`, `week-day-off.png` and `week-no-activity.png`.
pub fn capture_day_off(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let directory = demo::throwaway_directory();
    let worked = day_off(&directory, language, Behind::Worked);
    std::fs::remove_dir_all(&directory).ok();
    let (day, week) = worked?;
    let window = open_flyout(cx, day, Tracking::new(TrackingState::Running, false), None)?;
    capture(cx, window.into(), &output.join("flyout-day-off.png"))?;
    let window = cx.open_window(
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| {
            cx.new(|cx| {
                let mut view = WeekView::new(Some(week), language, window, cx);
                view.set_tracking(Tracking::new(TrackingState::Running, false));
                view
            })
        },
    )?;
    capture(cx, window.into(), &output.join("week-day-off.png"))?;

    let directory = demo::throwaway_directory();
    let empty = day_off(&directory, language, Behind::Nothing);
    std::fs::remove_dir_all(&directory).ok();
    let (_, week) = empty?;
    let window = cx.open_window(
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| cx.new(|cx| WeekView::new(Some(week), language, window, cx)),
    )?;
    capture(cx, window.into(), &output.join("week-no-activity.png"))
}

/// The day and the week of a launch on Saturday 10 October 2026 at 10:00 UTC, on `directory`,
/// with the built-in source as the onboarding selects it and `behind` it.
fn day_off(
    directory: &std::path::Path,
    language: Language,
    behind: Behind,
) -> anyhow::Result<(DayView, WeekTimeline)> {
    on_saturday(directory, behind, |host| {
        Ok((host.view(language), host.week()?))
    })
}
