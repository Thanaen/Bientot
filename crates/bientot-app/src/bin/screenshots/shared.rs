//! What several scenarios open or read: « Corriger le temps… », the flyout, a bubble of the
//! week and the week the demo scenarios store.

use anyhow::Context as _;
use bientot_app::adjust::window::Opener;
use bientot_app::adjust::{AdjustForm, Seed, Target};
use bientot_app::adjust_view::{self, AdjustView};
use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::Flyout;
use bientot_app::live::Tracking;
use bientot_app::strings::Language;
use bientot_app::theme::size::FLYOUT_WIDTH;
use bientot_app::week::{Answer, Segment, WeekState, WeekTimeline};
use bientot_app::week_view::WeekView;
use bientot_app::widgets::Notice;
use bientot_app::workday::DayView;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::{AdjustmentId, TrackingState};
use bientot_domain::time::LocalWorkday;
use gpui::{AppContext as _, HeadlessAppContext, WindowHandle, px, size};
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::capture;

/// A correction of a fixed day, so the images do not depend on the date.
pub(super) fn blank_form(language: Language) -> AdjustForm {
    let workday = LocalWorkday::new(jiff::civil::date(2026, 10, 8), "UTC");
    adjust_form(workday, Seed::Blank, jiff::Timestamp::UNIX_EPOCH, language)
}

/// « Corriger le temps… » of `workday` in UTC, opened on `seed` at `now`.
pub(super) fn adjust_form(
    workday: LocalWorkday,
    seed: Seed,
    now: jiff::Timestamp,
    language: Language,
) -> AdjustForm {
    let zone = TimeZone::UTC;
    let id = AdjustmentId::new("screenshot").expect("the id is not blank");
    AdjustForm::new(Target { workday, zone }, seed, id, now, language)
}

pub(super) fn open_adjust(
    cx: &mut HeadlessAppContext,
    form: AdjustForm,
    opener: Opener,
) -> anyhow::Result<WindowHandle<AdjustView>> {
    let bounds = size(px(adjust_view::WINDOW_WIDTH), px(form.window_height()));
    cx.open_window(bounds, |window, cx| {
        cx.new(|cx| AdjustView::new(form, opener, window, cx))
    })
}

/// The flyout of `day`, with `notice` above it. Its first frame measures the content and
/// resizes the window to it.
pub(super) fn capture_flyout(
    cx: &mut HeadlessAppContext,
    day: DayView,
    notice: Option<Notice>,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    let window = open_flyout(cx, day, working(TrackingState::Running), notice)?;
    capture(cx, window.into(), path)
}

/// `state` on a working day.
pub(super) fn working(state: TrackingState) -> Tracking {
    Tracking::new(state, true)
}

/// The flyout of `day` while tracking is `tracking`, opened with `notice` at rest above it.
pub(super) fn open_flyout(
    cx: &mut HeadlessAppContext,
    day: DayView,
    tracking: Tracking,
    notice: Option<Notice>,
) -> anyhow::Result<WindowHandle<Flyout>> {
    cx.open_window(size(FLYOUT_WIDTH, px(800.)), |window, cx| {
        cx.new(|cx| Flyout::new(day, tracking, notice, window, cx))
    })
}

/// The bubble of the first slot of `window`'s week that `matches`, which draws `what`.
pub(super) fn open_segment(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<WeekView>,
    what: &str,
    matches: impl Fn(&Segment) -> bool,
) -> anyhow::Result<()> {
    let opened = cx.update(|cx| {
        window.update(cx, |view, _, cx| {
            let state = view.state_mut()?;
            let segment = state.timeline.days.iter().find_map(|day| {
                let segment = day.segments.iter().find(|segment| matches(segment))?;
                Some(segment.id(day.date))
            })?;
            state.open(segment);
            cx.notify();
            state.bubble().map(|_| ())
        })
    })?;
    opened.with_context(|| format!("the history week draws {what}"))
}

/// `day` September 2026, in the week the demo scenarios store.
pub(super) fn demo_day(day: i8) -> Date {
    jiff::civil::date(2026, 9, day)
}

/// The week of `scenario` after `choices` in the bubbles « N » opens one after the other.
pub(super) fn read_demo_week(
    scenario: DemoScenario,
    language: Language,
    choices: &[Answer],
) -> anyhow::Result<WeekTimeline> {
    let directory = demo::throwaway_directory();
    let read = DemoHost::open(scenario, &directory).and_then(|mut host| {
        host.refresh(language)?;
        for choice in choices {
            let mut state = WeekState::new(host.week()?, language);
            state.settle_next();
            let command = state
                .choose(*choice)
                .ok_or_else(|| anyhow::anyhow!("{scenario:?} offers {choice:?}"))?;
            host.run_week_command(&command)
                .map_err(|key| anyhow::anyhow!("the demo refused {choice:?}: {key:?}"))?;
            host.refresh(language)?;
        }
        host.week()
    });
    std::fs::remove_dir_all(&directory).ok();
    read
}
