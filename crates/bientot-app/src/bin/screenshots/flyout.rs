//! The flyout of every demo scenario and tracking state, what it says unasked or once a
//! correction, a source switch or an export ended, the Store update and the command palette:
//! - `flyout-<scenario>.png`;
//! - `flyout-tracking-<paused|finished|paused-unavailable|finished-unavailable>.png`;
//! - `flyout-record-work-unavailable.png`, the hint of unrecorded time on a day that cannot be
//!   corrected, without « Oui, compter N min »;
//! - `flyout-problem-periods-focused.png`, its hint with « Oui, compter N min » focused;
//! - `flyout-unrecorded-counted.png`, once « Oui, compter N min » counted it;
//! - `flyout-unrecorded-counted-long.png`, the same notice telling of 1 h 26 min, on two lines;
//! - `flyout-appointment-review.png`, an absence in a recurring appointment to answer;
//! - `flyout-appointment-counted.png`, one counted automatically;
//! - `flyout-appointment-trust.png`, the offer to count one automatically;
//! - `flyout-adjust-<success|refresh-pending>.png`;
//! - `flyout-source-switch-failed.png`;
//! - `flyout-source-switch-failed-motion-<0|8|17|50|100>.png`, its entrance;
//! - `flyout-diagnostics-<saved|failed>.png`;
//! - `flyout-<settings-restored|notifications-unavailable|refresh-failed|alerts>.png`;
//! - `flyout-update-<offered|installing|failed>.png` and `flyout-update-direct-<…>.png`;
//! - `flyout-alerts-short.png` and `flyout-alerts-short-update.png`;
//! - `palette-flyout.png`, `palette-flyout-query.png` and `palette-flyout-empty.png`.

use anyhow::Context as _;
use bientot_app::automatic::{self, CountedAbsence, TrustOffer};
use bientot_app::demo::{self, DemoHost};
use bientot_app::diagnostics::Outcome;
use bientot_app::flyout::{Alerts, CountUnrecorded, Flyout};
use bientot_app::host::{Source, Today};
use bientot_app::live::{Announcement, Tracking};
use bientot_app::shell;
use bientot_app::strings::{Key, Language};
use bientot_app::theme::motion;
use bientot_app::theme::size::FLYOUT_WIDTH;
use bientot_app::time_text::minutes_text;
use bientot_app::tray_model::ClosedReview;
use bientot_app::updates::{DemoOffer, InstallFailure, Offer, Updates};
use bientot_app::widgets::Notice;
use bientot_app::workday::SourceHint;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::{AdjustmentId, LongLockReviewId, TrackingState};
use bientot_domain::adjustments::ManualAdjustment;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::time::LocalWorkday;
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use jiff::SignedDuration;

use super::shared::{blank_form, capture_flyout, open_flyout, working};
use super::{MENU_FRAMES, capture, capture_entrance};

pub fn capture_scenarios(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    for scenario in DemoScenario::ALL {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(scenario, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let day = day?;
        let path = output.join(format!("flyout-{}.png", demo::slug(scenario)));
        capture_flyout(cx, day, None, &path)?;
    }

    for (slug, scenario, tracking) in [
        ("paused", DemoScenario::OnTrack, TrackingState::Paused),
        ("finished", DemoScenario::Complete, TrackingState::Finished),
        (
            "paused-unavailable",
            DemoScenario::Unavailable,
            TrackingState::Paused,
        ),
        (
            "finished-unavailable",
            DemoScenario::Unavailable,
            TrackingState::Finished,
        ),
    ] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(scenario, &directory).and_then(|mut host| {
            // The source failed all day in this scenario: some work entered by hand gives the
            // known time a value a user would see.
            if scenario == DemoScenario::Unavailable {
                let declared = ManualAdjustment::duration_work(
                    AdjustmentId::new("declared")?,
                    LocalWorkday::new(host.today(), "UTC"),
                    jiff::SignedDuration::from_mins(5 * 60 + 10),
                    host.correction_time(),
                )?;
                host.save_adjustment(&declared)
                    .map_err(|key| anyhow::anyhow!("cannot declare the demo work: {key:?}"))?;
            }
            host.refresh(language)
        });
        std::fs::remove_dir_all(&directory).ok();
        let day = day?;
        let window = open_flyout(cx, day, working(tracking), None)?;
        let path = output.join(format!("flyout-tracking-{slug}.png"));
        capture(cx, window.into(), &path)?;
    }
    Ok(())
}

pub fn capture_record_work_unavailable(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // The day shown is no longer a working day by the calendar: its hint cannot record work.
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::ProblemPeriods, &directory)?.refresh(language);
    std::fs::remove_dir_all(&directory).ok();
    let tracking = Tracking::new(TrackingState::Running, false);
    let window = open_flyout(cx, day?, tracking, None)?;
    let path = output.join("flyout-record-work-unavailable.png");
    capture(cx, window.into(), &path)?;
    Ok(())
}

/// The hint of unrecorded time with « Oui, compter N min » focused by Tab, after « Ouvrir la
/// semaine » and « Paramètres »: its focus ring on the filled button.
pub fn capture_unrecorded_focused(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::ProblemPeriods, &directory)?.refresh(language);
    std::fs::remove_dir_all(&directory).ok();
    let window = open_flyout(cx, day?, working(TrackingState::Running), None)?;
    for _ in 0..3 {
        let keystroke = gpui::Keystroke::parse("tab")?;
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke,
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
    }
    capture(
        cx,
        window.into(),
        &output.join("flyout-problem-periods-focused.png"),
    )
}

pub fn capture_unrecorded_counted(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // « Oui, compter 26 min » of the problem periods' hint, as the day records it.
    let directory = demo::throwaway_directory();
    let counted = DemoHost::open(DemoScenario::ProblemPeriods, &directory).and_then(|host| {
        let mut today = Today::open(Source::Demo(Box::new(host)), language)?;
        let Some(SourceHint::Unrecorded {
            workday, intervals, ..
        }) = today.view().source_hint.clone()
        else {
            anyhow::bail!("the problem periods leave time unrecorded");
        };
        let notice = today.count_unrecorded(&CountUnrecorded { workday, intervals });
        Ok((today.view().clone(), notice))
    });
    std::fs::remove_dir_all(&directory).ok();
    let (day, notice) = counted?;
    // Past an hour, the notice wraps under its text, « Annuler » beside its first line.
    let long = Notice {
        message: language.format(
            Key::FlyoutUnrecordedCountedFormat,
            &[&minutes_text(SignedDuration::from_mins(86), language)],
        ),
        ..notice.clone()
    };
    capture_flyout(
        cx,
        day.clone(),
        Some(notice),
        &output.join("flyout-unrecorded-counted.png"),
    )?;
    capture_flyout(
        cx,
        day,
        Some(long),
        &output.join("flyout-unrecorded-counted-long.png"),
    )
}

pub fn capture_appointment_review(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory).and_then(|mut host| {
        let mut day = host.refresh(language)?;
        let today = host.today();
        let at = |hour, minute| {
            today
                .at(hour, minute, 0, 0)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .map(|zoned| zoned.timestamp())
        };
        let (start, end) = (at(14, 2)?, at(15, 47)?);
        let local = |instant: jiff::Timestamp| instant.to_zoned(jiff::tz::TimeZone::UTC).time();
        day.appointment_review = Some(ClosedReview {
            workday: LocalWorkday::new(today, "UTC"),
            id: LongLockReviewId::for_appointment("monday-1400", start),
            locked_episode: UtcInterval::new(start, end)?,
            local_start: local(start),
            local_end: local(end),
            finish_at: local(start),
            duration: end.duration_since(start),
            appointment: Some(String::new()),
        });
        Ok(day)
    });
    std::fs::remove_dir_all(&directory).ok();
    let path = output.join("flyout-appointment-review.png");
    capture_flyout(cx, day?, None, &path)?;

    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory).and_then(|mut host| {
        let mut day = host.refresh(language)?;
        let today = host.today();
        let at = |hour, minute| {
            today
                .at(hour, minute, 0, 0)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .map(|zoned| zoned.timestamp())
        };
        let (start, end) = (at(14, 2)?, at(15, 47)?);
        let local = |instant: jiff::Timestamp| instant.to_zoned(jiff::tz::TimeZone::UTC).time();
        let workday = LocalWorkday::new(today, "UTC");
        let review = LongLockReviewId::for_appointment("thursday-1400", start);
        day.counted_absence = Some(CountedAbsence {
            adjustment_id: automatic::count_id(&workday, &review),
            workday,
            interval: UtcInterval::new(start, end)?,
            local_start: local(start),
            local_end: local(end),
            duration: end.duration_since(start),
            appointment: "Réunion d’équipe".to_owned(),
        });
        Ok(day)
    });
    std::fs::remove_dir_all(&directory).ok();
    let path = output.join("flyout-appointment-counted.png");
    capture_flyout(cx, day?, None, &path)?;

    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory).and_then(|mut host| {
        let mut day = host.refresh(language)?;
        day.trust_offer = Some(TrustOffer {
            appointment_id: "thursday-1400".to_owned(),
            appointment: "Réunion d’équipe".to_owned(),
            weekday: jiff::civil::Weekday::Thursday,
            streak: automatic::TRUST_STREAK,
        });
        Ok(day)
    });
    std::fs::remove_dir_all(&directory).ok();
    let path = output.join("flyout-appointment-trust.png");
    capture_flyout(cx, day?, None, &path)?;
    Ok(())
}

pub fn capture_notices(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // What « Corriger le temps… » leaves in the flyout when the tray opened it.
    let mut saved = blank_form(language);
    saved.submit();
    let success = saved
        .finished(Ok(true))
        .context("the default correction saves")?
        .message;
    for (slug, notice) in [
        ("adjust-success", Notice::success(success)),
        (
            "adjust-refresh-pending",
            Notice::refresh_pending(language.text(Key::AdjustmentSavedRefreshPendingMessage)),
        ),
    ] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let path = output.join(format!("flyout-{slug}.png"));
        capture_flyout(cx, day?, Some(notice), &path)?;
    }

    // « Utiliser la détection intégrée » failed to save, « Paramètres » closed.
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::ActivityWatchUnavailable, &directory)?.refresh(language);
    std::fs::remove_dir_all(&directory).ok();
    let failure = Notice::source_switch_failed(language.text(Key::ActivitySourceSwitchFailed));
    let day = day?;
    capture_flyout(
        cx,
        day.clone(),
        Some(failure.clone()),
        &output.join("flyout-source-switch-failed.png"),
    )?;
    // The same failure answered while the flyout is open: it comes in.
    let window = open_flyout(cx, day, working(TrackingState::Running), None)?;
    cx.update_window(window.into(), |_, window, _| window.refresh())?;
    cx.run_until_parked();
    cx.update(|cx| window.update(cx, |flyout, _, cx| flyout.switched(Some(failure), cx)))?;
    capture_entrance(
        cx,
        window.into(),
        motion::MENU,
        MENU_FRAMES,
        output,
        "flyout-source-switch-failed",
    )?;

    // What « Exporter le diagnostic… » leaves in the flyout.
    for (slug, outcome) in [("saved", Outcome::Saved), ("failed", Outcome::Failed)] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let path = output.join(format!("flyout-diagnostics-{slug}.png"));
        capture_flyout(cx, day?, outcome.notice(language), &path)?;
    }

    // What the flyout tells unasked: the settings restored and the notifications refused at the
    // start, a refresh that failed, then all three.
    let restored = Announcement::SettingsRecovered.notice(language);
    let refused = Announcement::NotificationsRefused.notice(language);
    let failed = Notice::refresh_pending(language.text(Key::PeriodicRefreshErrorMessage));
    for (slug, announcements, refresh_failure) in [
        ("settings-restored", vec![restored.clone()], None),
        ("notifications-unavailable", vec![refused.clone()], None),
        ("refresh-failed", Vec::new(), Some(failed.clone())),
        ("alerts", vec![restored, refused], Some(failed)),
    ] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let day = day?;
        let alerts = Alerts {
            announcements,
            refresh_failure,
        };
        let window = cx.open_window(size(FLYOUT_WIDTH, px(800.)), |window, cx| {
            cx.new(|cx| {
                Flyout::new(day, working(TrackingState::Running), None, window, cx)
                    .with_alerts(alerts)
            })
        })?;
        let path = output.join(format!("flyout-{slug}.png"));
        capture(cx, window.into(), &path)?;
    }
    Ok(())
}

pub fn capture_updates(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // The Store update, found while the flyout is open: offered, installing, then failed; then
    // the downloaded update of the installed build, offered, restarting and failed.
    let plain = image::image_dimensions(output.join("flyout-on-track.png"))?.1;
    for (restarts, state) in [
        (false, "offered"),
        (false, "installing"),
        (false, "failed"),
        (true, "offered"),
        (true, "installing"),
        (true, "failed"),
    ] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let day = day?;
        let window = open_flyout(cx, day, working(TrackingState::Running), None)?;
        cx.run_until_parked();
        cx.update(|cx| {
            let mut updates = Updates::default();
            let offer = Offer::new(DemoOffer);
            updates.found(if restarts { offer.restarting() } else { offer });
            if state != "offered" {
                updates.install();
            }
            if state == "failed" {
                updates.installed(&Err(InstallFailure {
                    error_type: "screenshot",
                    message: "the Store refused".to_owned(),
                }));
            }
            cx.set_global(updates);
        });
        let slug = if restarts {
            format!("direct-{state}")
        } else {
            state.to_owned()
        };
        let path = output.join(format!("flyout-update-{slug}.png"));
        capture(cx, window.into(), &path)?;
        let height = image::image_dimensions(&path)?.1;
        anyhow::ensure!(
            height > plain,
            "the open flyout did not show the {state} update ({height} px, {plain} px without)"
        );
    }
    cx.update(|cx| cx.set_global(Updates::default()));

    // Every alert and the update on a 1366 × 768 screen at 125 %, above the taskbar: the body
    // scrolls, then Maj+Tab from the actions brings « Mettre à jour » into view.
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
    std::fs::remove_dir_all(&directory).ok();
    let day = day?;
    cx.update(|cx| {
        let mut updates = Updates::default();
        updates.found(Offer::new(DemoOffer));
        cx.set_global(updates);
    });
    let alerts = Alerts {
        announcements: vec![
            Announcement::SettingsRecovered.notice(language),
            Announcement::NotificationsRefused.notice(language),
        ],
        refresh_failure: Some(Notice::refresh_pending(
            language.text(Key::PeriodicRefreshErrorMessage),
        )),
    };
    let window = cx.open_window(size(FLYOUT_WIDTH, px(800.)), |window, cx| {
        cx.new(|cx| {
            Flyout::new(day, working(TrackingState::Running), None, window, cx)
                .with_alerts(alerts)
                .with_max_height(shell::panel_max_height(708, 1.25))
        })
    })?;
    capture(cx, window.into(), &output.join("flyout-alerts-short.png"))?;
    for _ in 0..4 {
        let keystroke = gpui::Keystroke::parse("shift-tab")?;
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke,
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
    }
    capture(
        cx,
        window.into(),
        &output.join("flyout-alerts-short-update.png"),
    )?;
    cx.update(|cx| cx.set_global(Updates::default()));
    Ok(())
}

pub fn capture_palette(
    cx: &mut HeadlessAppContext,
    language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    // The command palette over the flyout: as Ctrl+Maj+P opens it, filtered, matching nothing.
    let query = match language {
        Language::French => "par",
        Language::English => "set",
    };
    for (slug, query) in [("", None), ("-query", Some(query)), ("-empty", Some("zzz"))] {
        let directory = demo::throwaway_directory();
        let day = DemoHost::open(DemoScenario::OnTrack, &directory)?.refresh(language);
        std::fs::remove_dir_all(&directory).ok();
        let day = day?;
        let window = open_flyout(cx, day, working(TrackingState::Running), None)?;
        cx.update(|cx| {
            window.update(cx, |flyout, window, cx| {
                flyout.open_palette(window, cx);
                if let (Some(query), Some(palette)) = (query, flyout.palette()) {
                    palette.update(cx, |palette, cx| palette.type_query(query, cx));
                }
            })
        })?;
        let path = output.join(format!("palette-flyout{slug}.png"));
        capture(cx, window.into(), &path)?;
    }
    Ok(())
}
