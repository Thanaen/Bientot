//! « Détails de l’activité et du crédit » of the history week: `week-history-week-details.png`
//! on Tuesday's meeting, `-corrections.png` on Wednesday once its long lock is confirmed and its
//! gap counted, the confirmation locked, and `-debt.png` on Thursday, its shortfall owed as debt.
//! `-bound.png` is Wednesday once its long lock is excluded, the correction bound to its review.
//! `-no-summary.png` leaves them open on Tuesday of the problem periods, a day without a summary
//! and so without the toggle. `-narrow.png` and `-short.png` show Wednesday's corrections in a
//! window at the week's minimum width, the credit above them, and in one at its minimum height.

use bientot_app::strings::Language;
use bientot_app::theme::size as token;
use bientot_app::week::{Answer, WeekTimeline};
use bientot_app::week_view::WeekView;
use bientot_core::demo::DemoScenario;
use gpui::{AppContext as _, HeadlessAppContext, Pixels, Size, size};
use jiff::civil::Date;

use super::capture;
use super::shared::{demo_day, read_demo_week};

pub fn capture_details(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let opening = size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT);
    let week = read_demo_week(DemoScenario::HistoryWeek, language, &[])?;
    shoot(cx, week.clone(), demo_day(1), opening, language, output, "")?;
    let mut owing = week;
    if let Some(figures) = owing
        .days
        .iter_mut()
        .find(|shown| shown.date == demo_day(3))
        .and_then(|shown| shown.figures.as_mut())
    {
        // Its last hour of credit applied, Thursday still ends short of its target: with debt
        // on, the week owes that shortfall.
        figures.credit.balance = figures.credited - figures.effective_target;
    }
    shoot(cx, owing, demo_day(3), opening, language, output, "-debt")?;
    let settled = read_demo_week(
        DemoScenario::HistoryWeek,
        language,
        &[Answer::Count, Answer::Count],
    )?;
    shoot(
        cx,
        settled.clone(),
        demo_day(2),
        opening,
        language,
        output,
        "-corrections",
    )?;
    let narrow = size(token::WEEK_WINDOW_MIN_WIDTH, token::WEEK_WINDOW_HEIGHT);
    shoot(
        cx,
        settled.clone(),
        demo_day(2),
        narrow,
        language,
        output,
        "-narrow",
    )?;
    let short = size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_MIN_HEIGHT);
    shoot(cx, settled, demo_day(2), short, language, output, "-short")?;
    let bound = read_demo_week(DemoScenario::HistoryWeek, language, &[Answer::Exclude])?;
    shoot(cx, bound, demo_day(2), opening, language, output, "-bound")?;
    let problems = read_demo_week(DemoScenario::ProblemPeriods, language, &[])?;
    shoot(
        cx,
        problems,
        demo_day(1),
        opening,
        language,
        output,
        "-no-summary",
    )
}

fn shoot(
    cx: &mut HeadlessAppContext,
    week: WeekTimeline,
    open: Date,
    window_size: Size<Pixels>,
    language: Language,
    output: &std::path::Path,
    suffix: &str,
) -> anyhow::Result<()> {
    let window = cx.open_window(window_size, |window, cx| {
        cx.new(|cx| WeekView::new(Some(week), language, window, cx))
    })?;
    cx.update(|cx| {
        window.update(cx, |view, _, cx| {
            if let Some(state) = view.state_mut() {
                state.select(open);
            }
            view.toggle_details(cx);
        })
    })?;
    capture(
        cx,
        window.into(),
        &output.join(format!("week-history-week-details{suffix}.png")),
    )
}
