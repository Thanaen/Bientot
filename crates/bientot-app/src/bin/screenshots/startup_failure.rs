//! The window of a start that failed, before, during and after « Exporter le diagnostic… »:
//! `startup-failure[-exporting|-saved|-failed|-long-failed[-scrolled]].png`.

use bientot_app::diagnostics::Outcome;
use bientot_app::startup_failure::StartupFailure;
use bientot_app::startup_failure_view::{self, StartupFailureView};
use bientot_app::strings::Language;
use gpui::{AnyWindowHandle, AppContext as _, HeadlessAppContext, px, size};

use super::capture;

pub fn capture_failures(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for (slug, chain, export) in [
        ("", LEDGER_ERROR.as_slice(), None),
        ("-exporting", &LEDGER_ERROR, Some(None)),
        ("-saved", &LEDGER_ERROR, Some(Some(Outcome::Saved))),
        ("-failed", &LEDGER_ERROR, Some(Some(Outcome::Failed))),
        (
            "-long-failed",
            &MIGRATION_ERROR,
            Some(Some(Outcome::Failed)),
        ),
    ] {
        let window = open_startup_failure(cx, language, chain, export)?;
        capture(
            cx,
            window,
            &output.join(format!("startup-failure{slug}.png")),
        )?;
    }
    // The details scrolled to their last cause, under the wheel.
    let window = open_startup_failure(cx, language, &MIGRATION_ERROR, Some(Some(Outcome::Failed)))?;
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                position: gpui::point(px(260.), px(190.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-400.))),
                modifiers: gpui::Modifiers::default(),
                touch_phase: gpui::TouchPhase::Moved,
            }),
            cx,
        );
    })?;
    capture(
        cx,
        window,
        &output.join("startup-failure-long-failed-scrolled.png"),
    )?;
    Ok(())
}

/// A start whose ledger is not a database: the error, then each cause.
const LEDGER_ERROR: [&str; 3] = [
    r"cannot open the ledger C:\Users\Camille\AppData\Local\Bientot\worktime.db",
    "SQLite failed: file is not a database",
    "Error code 26: file is not a database",
];

/// A start whose ledger did not migrate, with more causes than the details show at once.
const MIGRATION_ERROR: [&str; 6] = [
    r"cannot open the ledger C:\Users\Camille\AppData\Local\Bientot\worktime.db",
    "cannot migrate the ledger from schema 6 to schema 7",
    "migration 7 failed: ALTER TABLE activity_spans ADD COLUMN source_kind",
    "cannot write the ledger's journal",
    "SQLite failed: database disk image is malformed",
    "Error code 11: database disk image is malformed",
];

/// The window of a start that failed with `chain`, the error then each cause: before
/// « Exporter le diagnostic… » when `export` is `None`, else while it runs or once it ended with
/// the outcome given.
fn open_startup_failure(
    cx: &mut HeadlessAppContext,
    language: Language,
    chain: &[&'static str],
    export: Option<Option<Outcome>>,
) -> anyhow::Result<AnyWindowHandle> {
    let (root, contexts) = chain.split_last().expect("an error");
    let error = contexts
        .iter()
        .rev()
        .fold(anyhow::anyhow!(*root), |error, context| {
            error.context(*context)
        });
    let mut failure = StartupFailure::new(&error, None, Some(language));
    if let Some(outcome) = export {
        failure.start_export();
        if let Some(outcome) = outcome {
            failure.exported(outcome);
        }
    }
    let window = cx.open_window(
        size(
            startup_failure_view::WINDOW_WIDTH,
            startup_failure_view::WINDOW_HEIGHT,
        ),
        |window, cx| cx.new(|cx| StartupFailureView::new(failure, window, cx)),
    )?;
    Ok(window.into())
}
