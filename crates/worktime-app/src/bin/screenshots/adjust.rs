//! « Corriger le temps… » in each state, declaring work and editing a correction:
//! - `adjust-<intent>-<state>.png`, each state;
//! - `adjust-add-interval-long-reason.png`, an added interval with a long reason;
//! - `adjust-declare-work.png`, declaring the unrecorded time of the week's day, in its day;
//! - `adjust-declare-work-other-slot.png`, the same on its other slot;
//! - `adjust-declare-work-interval.png` and `adjust-declare-work-blank.png`, declaring work on
//!   an interval with no day around it, and on none;
//! - `adjust-declare-work-refresh-pending.png`, a declaration saved before the totals refresh;
//! - `flyout-adjust-success-seconds.png`, the flyout once a declaration is saved;
//! - `adjust-edit-add-<interval|duration>.png`, editing a correction.

use anyhow::Context as _;
use gpui::HeadlessAppContext;
use worktime_app::adjust::window::Opener;
use worktime_app::adjust::{AdjustForm, Editable, Intent, Seed, TimeField};
use worktime_app::adjust_view::AdjustView;
use worktime_app::demo::{self, DemoHost};
use worktime_app::strings::{Key, Language};
use worktime_app::week::WeekState;
use worktime_app::widgets::Notice;
use worktime_core::demo::DemoScenario;
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::adjustments::ManualClassification;
use worktime_domain::time::LocalWorkday;

use super::shared::{adjust_form, blank_form, capture_flyout, open_adjust};
use super::{HISTORY_WEEK_MANUAL, HISTORY_WEEK_MEETING, capture};

pub fn capture_states(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for (intent, state) in [
        (Intent::AddInterval, State::Pristine),
        (Intent::AddInterval, State::Invalid),
        (Intent::AddDuration, State::Invalid),
        (Intent::Exclude, State::Saving),
        (Intent::Reclassify, State::Pristine),
        (Intent::Reclassify, State::Failed),
        (Intent::AddInterval, State::RefreshPending),
        (Intent::Reclassify, State::RefreshPending),
    ] {
        let window = open_adjust(cx, blank_form(language), Opener::Week)?;
        cx.update(|cx| {
            window.update(cx, |view, _, cx| {
                state.apply(intent, view, cx);
                cx.notify();
            })
        })?;
        let name = format!("adjust-{}-{}.png", intent_slug(intent), state.slug());
        capture(cx, window.into(), &output.join(name))?;
    }

    // A reason wider than its field, the caret at its end: the line scrolls to keep it in view.
    let window = open_adjust(cx, blank_form(language), Opener::Week)?;
    cx.update(|cx| {
        window.update(cx, |view, window, cx| {
            view.set_reason(long_reason(language), window, cx);
        })
    })?;
    capture(
        cx,
        window.into(),
        &output.join("adjust-add-interval-long-reason.png"),
    )?;
    Ok(())
}

pub fn capture_declare_work(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // « Renseigner un créneau » of the week: on the problem periods, whose two unrecorded
    // intervals leave the times to the user, on the longest, then on the other.
    let directory = demo::throwaway_directory();
    let mut host = DemoHost::open(DemoScenario::ProblemPeriods, &directory)?;
    host.refresh(language)?;
    let week = host.week();
    let now = host.correction_time();
    drop(host);
    std::fs::remove_dir_all(&directory).ok();
    let declaration = WeekState::new(week?, language)
        .declare_work()
        .context("the problem periods leave time unrecorded")?;
    let (_, Seed::DeclareWork(Some(unrecorded))) = declaration else {
        anyhow::bail!("the problem periods leave time unrecorded");
    };
    for (slug, seed, chosen) in [
        (
            "adjust-declare-work",
            Seed::DeclareWork(Some(unrecorded.clone())),
            None,
        ),
        (
            "adjust-declare-work-other-slot",
            Seed::DeclareWork(Some(unrecorded.clone())),
            Some(0),
        ),
        ("adjust-declare-work-blank", Seed::DeclareWork(None), None),
    ] {
        let workday = LocalWorkday::new(jiff::civil::date(2026, 8, 31), "UTC");
        let mut form = adjust_form(workday, seed, now, language);
        if let Some(index) = chosen {
            form.choose_slot(index);
        }
        let window = open_adjust(cx, form, Opener::Flyout)?;
        capture(cx, window.into(), &output.join(format!("{slug}.png")))?;
    }
    let workday = LocalWorkday::new(jiff::civil::date(2026, 8, 31), "UTC");
    let first = Seed::DeclareWork(Some(unrecorded));
    // Saved, the totals not refreshed: the tallest declaration, which its window fits.
    let mut pending = adjust_form(workday.clone(), first.clone(), now, language);
    pending
        .submit()
        .context("the first unrecorded interval is declared as it opens")?;
    pending.finished(Ok(false));
    let window = open_adjust(cx, pending, Opener::Flyout)?;
    capture(
        cx,
        window.into(),
        &output.join("adjust-declare-work-refresh-pending.png"),
    )?;
    let declared = adjust_form(workday, first, now, language);
    capture_success_seconds(cx, declared, language, output)?;
    Ok(())
}

pub fn capture_edits(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // « Modifier… » on the two corrections of the history week: Tuesday's meeting, then
    // Monday's half hour.
    let directory = demo::throwaway_directory();
    let mut host = DemoHost::open(DemoScenario::HistoryWeek, &directory)?;
    let edits = host.refresh(language).and_then(|_| {
        let today = host.today();
        [
            ("adjust-edit-add-interval", HISTORY_WEEK_MEETING, 3),
            ("adjust-edit-add-duration", HISTORY_WEEK_MANUAL, 4),
        ]
        .into_iter()
        .map(|(slug, id, days_before)| {
            let workday = LocalWorkday::new(today - jiff::Span::new().days(days_before), "UTC");
            let edited = host
                .active_adjustment(&workday, &AdjustmentId::new(id)?)
                .ok()
                .flatten()
                .and_then(Editable::new)
                .with_context(|| format!("the history week holds {id}"))?;
            let form = adjust_form(
                workday,
                Seed::Edit(edited),
                host.correction_time(),
                language,
            );
            Ok((slug, form))
        })
        .collect::<anyhow::Result<Vec<_>>>()
    });
    drop(host);
    std::fs::remove_dir_all(&directory).ok();
    for (slug, form) in edits? {
        let window = open_adjust(cx, form, Opener::Week)?;
        capture(cx, window.into(), &output.join(format!("{slug}.png")))?;
    }
    Ok(())
}

/// The states of the mockup's « Corriger le temps… » board.
#[derive(Clone, Copy)]
enum State {
    Pristine,
    Invalid,
    Saving,
    Failed,
    RefreshPending,
}

impl State {
    fn slug(self) -> &'static str {
        match self {
            Self::Pristine => "pristine",
            Self::Invalid => "invalid",
            Self::Saving => "saving",
            Self::Failed => "failed",
            Self::RefreshPending => "refresh-pending",
        }
    }

    fn apply(self, intent: Intent, view: &mut AdjustView, cx: &mut gpui::Context<AdjustView>) {
        view.set_duration(
            match self {
                Self::Pristine => "",
                Self::Invalid => "0:00",
                _ => "1:30",
            },
            cx,
        );
        let form = view.form_mut();
        form.set_intent(intent);
        if let Self::Failed = self {
            form.set_classification(ManualClassification::Excluded);
        }
        if let Self::Invalid = self {
            form.start = TimeField::new(10, 0);
            form.end = TimeField::new(9, 30);
        }
        if let Self::Pristine = self {
            return;
        }
        form.submit();
        match self {
            Self::Failed => {
                form.finished(Err(Key::AdjustTimeErrorSubmissionFailed));
            }
            Self::RefreshPending => {
                form.finished(Ok(false));
            }
            _ => {}
        }
    }
}

/// `flyout-adjust-success-seconds.png`: the flyout once `declared`, the first unrecorded
/// interval of the problem periods, is saved as it opens, which reports it to the second.
fn capture_success_seconds(
    cx: &mut HeadlessAppContext,
    mut declared: AdjustForm,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    declared
        .submit()
        .context("the first unrecorded interval is declared as it opens")?;
    let success = declared
        .finished(Ok(true))
        .context("the declaration saves")?
        .message;
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
    std::fs::remove_dir_all(&directory).ok();
    let path = output.join("flyout-adjust-success-seconds.png");
    capture_flyout(cx, day?, Some(Notice::success(success)), &path)
}

fn long_reason(language: Language) -> &'static str {
    match language {
        Language::French => {
            "Réunion client prolongée à l’extérieur, poste resté verrouillé jusqu’au retour au bureau"
        }
        Language::English => {
            "Client meeting ran long off site, the workstation stayed locked until back at the office"
        }
    }
}

fn intent_slug(intent: Intent) -> &'static str {
    match intent {
        Intent::AddInterval => "add-interval",
        Intent::AddDuration => "add-duration",
        Intent::Exclude => "exclude",
        Intent::Reclassify => "reclassify",
    }
}
