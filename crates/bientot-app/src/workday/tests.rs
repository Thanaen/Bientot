//! The flyout of the demo days, calculated by the real accounting over a migrated ledger.

use bientot_core::accounting::ActivityReadDisposition;
use bientot_core::demo::DemoScenario;
use bientot_core::weekly::SourceEvidenceUnavailable;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::SourceDiagnostics;
use jiff::civil::date;

use super::*;
use crate::demo::DemoHost;

fn day(scenario: DemoScenario, language: Language) -> DayView {
    let directory = tempfile::tempdir().unwrap();
    DemoHost::open(scenario, directory.path())
        .unwrap()
        .refresh(language)
        .unwrap()
}

/// `text` with its no-break spaces shown as `_`, so the expectations show where lines may break.
fn spaced(text: &str) -> String {
    text.replace('\u{a0}', "_")
}

fn summary_of(view: &DayView) -> Vec<String> {
    let headline = &view.headline;
    let worked = view.worked.as_ref();
    [
        Some(format!("{:?} {}", view.tone, view.status)),
        Some(format!(
            "{} {} {}",
            headline.label.as_deref().unwrap_or("-"),
            headline.value,
            if headline.emphasized {
                "(large)"
            } else {
                "(message)"
            }
        )),
        headline.hint.as_ref().map(|hint| hint.to_string()),
        headline.lunch.as_ref().map(|lunch| lunch.to_string()),
        headline
            .qualifier
            .as_ref()
            .map(|qualifier| qualifier.to_string()),
        worked.map(|worked| match &worked.target {
            Some(target) => format!(
                "{} {} / {} at {:.2}",
                worked.label, worked.worked, target.duration, target.progress
            ),
            None => format!("{} {}", worked.label, worked.worked),
        }),
        view.credit
            .as_ref()
            .map(|credit| format!("{} {}", credit.label, credit.value)),
        view.source_hint.as_ref().map(|hint| match hint {
            SourceHint::Failure { message, remedy } => format!("{remedy:?}: {message}"),
            SourceHint::Unrecorded { message, .. } => format!("Unrecorded: {message}"),
        }),
        Some(format!("tray: {}", view.tray_tooltip)),
    ]
    .into_iter()
    .flatten()
    .map(|line| spaced(&line))
    .collect()
}

#[test]
fn on_track_shows_the_departure_and_the_margin_left() {
    assert_eq!(
        summary_of(&day(DemoScenario::OnTrack, Language::French)),
        [
            "OnTrack Dans les temps",
            "Départ prévu 17:00 (large)",
            "Marge de 1_h_30_min avant votre heure limite (18:30)",
            "Temps travaillé 6_h_00_min / 7_h_00_min at 0.86",
            "tray: Dans les temps · 6_h_00_min/7_h_00_min · Départ 17:00",
        ]
    );
    assert_eq!(
        summary_of(&day(DemoScenario::OnTrack, Language::English)),
        [
            "OnTrack On track",
            "Projected departure 5:00_PM (large)",
            "1h_30m to spare before your latest departure (6:30_PM)",
            "Time worked 6h_00m / 7h_00m at 0.86",
            "tray: On track · 6h_00m/7h_00m · Departure 5:00_PM",
        ]
    );
}

#[test]
fn late_shows_how_far_the_departure_overruns() {
    assert_eq!(
        summary_of(&day(DemoScenario::Late, Language::French)),
        [
            "Late En retard",
            "Départ prévu 19:00 (large)",
            "Dépassement de 30_min sur votre heure limite (18:30)",
            "Temps travaillé 4_h_00_min / 7_h_00_min at 0.57",
            "tray: Départ après la limite · 4_h_00_min/7_h_00_min · Départ 19:00",
        ]
    );
}

#[test]
fn a_complete_day_shows_the_overtime() {
    assert_eq!(
        summary_of(&day(DemoScenario::Complete, Language::French)),
        [
            "Complete Objectif atteint",
            "Heures supplémentaires 1_h_00_min (large)",
            "Ce temps s’ajoute à votre crédit de la semaine",
            "Temps travaillé 8_h_00_min / 7_h_00_min at 1.00",
            "tray: Objectif atteint · 8_h_00_min/7_h_00_min · Heures sup. 1_h_00_min",
        ]
    );
}

#[test]
fn before_lunch_ends_the_departure_leaves_lunch_free() {
    assert_eq!(
        summary_of(&day(DemoScenario::Morning, Language::English)),
        [
            "OnTrack On track",
            "Projected departure 6:00_PM (large)",
            "30m to spare before your latest departure (6:30_PM)",
            "Lunch break planned until 2:00_PM",
            "Time worked 2h_00m / 7h_00m at 0.29",
            "tray: On track · 2h_00m/7h_00m · Departure 6:00_PM",
        ]
    );
}

#[test]
fn an_unavailable_demo_source_reads_as_the_built_in_one() {
    let lines = summary_of(&day(DemoScenario::Unavailable, Language::French));
    assert_eq!(lines[0], "SourceProblem Suivi à vérifier");
    assert_eq!(
        lines[1..3],
        [
            "Départ prévu Prévision indisponible (message)",
            "Recalculée dès que le suivi reprend",
        ]
    );
    assert!(lines[3].starts_with("Temps connu "), "{lines:?}");
    assert_eq!(
        lines[4..],
        [
            "OpenSettings: Bientôt ne parvient pas à lire votre dernière utilisation du clavier ou \
             de la souris. Ce temps reste non renseigné. La lecture reprend d’elle-même.",
            "tray: Lecture des saisies impossible · Ouvrez l’application",
        ]
    );
}

#[test]
fn an_estimated_forecast_says_what_it_is_based_on() {
    assert_eq!(
        summary_of(&day(DemoScenario::ProblemPeriods, Language::French)),
        [
            "OnTrack Dans les temps",
            "Départ prévu 17:55 (large)",
            "Marge de 34_min avant votre heure limite (18:30)",
            "Selon le travail enregistré",
            "Temps travaillé 5_h_04_min / 7_h_00_min at 0.73",
            "Unrecorded: Temps non renseigné · 26_min en 2_créneaux, entre 09:40 et 14:20. \
             Travailliez-vous\u{202f}?",
            "tray: Dans les temps · 5_h_04_min/7_h_00_min · Départ 17:55 · Selon le travail enregistré",
        ]
    );
}

/// `SourceDiagnosticsPresenter.Present` of the .NET app: the unknown time the source could
/// not record, merged, to the minute, and whether it was work. The message also says when the
/// time was (#301 follow-up), and the hint keeps its day and how long it lasts.
#[test]
fn unrecorded_time_asks_whether_it_was_work() {
    let morning = UtcInterval::new(
        "2026-08-31T09:40:12Z".parse().unwrap(),
        "2026-08-31T09:45:42Z".parse().unwrap(),
    )
    .unwrap();
    let afternoon = UtcInterval::new(
        "2026-08-31T14:00:00Z".parse().unwrap(),
        "2026-08-31T14:20:00Z".parse().unwrap(),
    )
    .unwrap();
    let Some(SourceHint::Unrecorded {
        message,
        workday,
        intervals,
        duration,
        ..
    }) = &day(DemoScenario::ProblemPeriods, Language::English).source_hint
    else {
        panic!("the problem periods leave time unrecorded");
    };
    assert_eq!(
        message,
        "Unrecorded time · 26m in 2\u{a0}slots, between 9:40\u{a0}AM and 2:20\u{a0}PM. Were you \
         working?"
    );
    assert_eq!(workday.date, jiff::civil::date(2026, 8, 31));
    assert_eq!(intervals, &[morning, afternoon]);
    assert_eq!(*duration, SignedDuration::new(25 * 60 + 30, 0));
}

/// On a day off, where the flyout leaves « Oui, compter » out, the hint no longer asks the
/// question it answers.
#[test]
fn the_hint_asks_whether_you_were_working_only_where_it_can_count() {
    let hint = day(DemoScenario::ProblemPeriods, Language::French)
        .source_hint
        .expect("the problem periods leave time unrecorded");
    let fact = "Temps non renseigné · 26\u{a0}min en 2\u{a0}créneaux, entre 09:40 et 14:20.";
    assert_eq!(hint.text(false), fact);
    assert_eq!(
        hint.text(true),
        &format!("{fact} Travailliez-vous\u{202f}?")
    );
}

/// Monday's hour of credit lowers Tuesday's target to 6 hours and leaves none to carry over;
/// with 12 minutes worked by 16:00, the departure falls after the limit.
#[test]
fn the_credit_day_spends_monday_s_hour_on_its_target() {
    assert_eq!(
        summary_of(&day(DemoScenario::Credit, Language::French)),
        [
            "Late En retard",
            "Départ prévu 21:48 (large)",
            "Dépassement de 3_h_18_min sur votre heure limite (18:30)",
            "Temps travaillé 12_min / 6_h_00_min at 0.03",
            "tray: Départ après la limite · 12_min/6_h_00_min · Départ 21:48",
        ]
    );
}

/// Tuesday to Thursday spent the week's 3 hours of credit. Friday's 2 hours over the target
/// include the 4 hours locked since 14:00, which await review and so earn no credit, so the
/// flyout does not promise any.
#[test]
fn the_stored_friday_has_overtime_but_no_credit_left() {
    assert_eq!(
        summary_of(&day(DemoScenario::HistoryWeek, Language::French)),
        [
            "Complete Objectif atteint",
            "Heures supplémentaires 2_h_00_min (large)",
            "Temps travaillé 9_h_00_min / 7_h_00_min at 1.00",
            "tray: Objectif atteint · 9_h_00_min/7_h_00_min · Heures sup. 2_h_00_min",
        ]
    );
}

/// The day of 8 October 2026 at 15:30 UTC, whose source `source_id` did not answer.
fn unavailable_source(source_id: &str) -> DayView {
    source_in_trouble(source_id, SourceFreshness::Unavailable)
}

/// A source reporting incomplete data, `freshness` as given, before any summary.
fn source_in_trouble(source_id: &str, freshness: SourceFreshness) -> DayView {
    let bounds = UtcInterval::new(
        date(2026, 10, 8)
            .at(0, 0, 0, 0)
            .in_tz("UTC")
            .unwrap()
            .timestamp(),
        date(2026, 10, 9)
            .at(0, 0, 0, 0)
            .in_tz("UTC")
            .unwrap()
            .timestamp(),
    )
    .unwrap();
    let health = SourceHealth::degraded(
        source_id,
        freshness,
        SourceQuality::Incomplete,
        "the source did not answer",
        None,
    )
    .unwrap();
    let state = WorkdayRefreshState {
        summary: None,
        source_failure: Some(SourceEvidenceUnavailable {
            workday_date: date(2026, 10, 8),
            diagnostics: SourceDiagnostics::new(health, bounds, bounds.end(), []).unwrap(),
            disposition: ActivityReadDisposition::Unavailable,
        }),
    };

    DayView::present(
        &state,
        &Settings::default(),
        &TimeZone::UTC,
        Language::French,
    )
}

#[test]
fn a_failed_built_in_source_names_its_cause_in_the_tray() {
    let view = unavailable_source(BuiltInActivitySource::SOURCE_ID);

    assert_eq!(
        summary_of(&view),
        [
            "SourceProblem Suivi à vérifier",
            "Départ prévu Prévision indisponible (message)",
            "Recalculée dès que le suivi reprend",
            "OpenSettings: Bientôt ne parvient pas à lire votre dernière utilisation du clavier ou \
             de la souris. Ce temps reste non renseigné. La lecture reprend d’elle-même.",
            "tray: Lecture des saisies impossible · Ouvrez l’application",
        ]
    );
    assert_eq!(view.tray_icon.file_stem(), "indicator-green-degraded");
}

#[test]
fn an_activitywatch_source_in_trouble_offers_the_built_in_detection() {
    let view = unavailable_source(ACTIVITY_WATCH_ID);

    assert_eq!(
        view.source_hint,
        Some(SourceHint::Failure {
            message:
                "ActivityWatch ne répond pas, et Bientôt ne le prend plus en charge. Passez à la \
                      détection intégrée pour que votre temps continue d’être compté."
                    .into(),
            remedy: FailureRemedy::UseBuiltIn,
        })
    );
}

#[test]
fn activitywatch_without_usable_tracking_points_to_the_built_in_detection() {
    let view = source_in_trouble(ACTIVITY_WATCH_ID, SourceFreshness::Fresh);

    assert_eq!(
        view.source_hint,
        Some(SourceHint::Failure {
            message: "Aucun suivi d’activité utilisable n’a été trouvé. Bientôt ne prend plus en \
                      charge ActivityWatch\u{a0}: passez à la détection intégrée."
                .into(),
            remedy: FailureRemedy::UseBuiltIn,
        })
    );
}

#[test]
fn the_tray_icon_keeps_the_day_s_colour_and_badges_a_source_in_trouble() {
    let icons: Vec<_> = DemoScenario::ALL
        .into_iter()
        .map(|scenario| {
            (
                scenario,
                day(scenario, Language::French).tray_icon.file_stem(),
            )
        })
        .collect();

    assert_eq!(
        icons,
        [
            (DemoScenario::OnTrack, "indicator-green"),
            (DemoScenario::Warning, "indicator-orange"),
            (DemoScenario::Late, "indicator-red"),
            (DemoScenario::Complete, "indicator-blue"),
            (DemoScenario::Morning, "indicator-green"),
            (DemoScenario::Unavailable, "indicator-red-degraded"),
            (DemoScenario::Credit, "indicator-red"),
            (DemoScenario::HistoryWeek, "indicator-blue"),
            (DemoScenario::MorningWarning, "indicator-orange"),
            (DemoScenario::MorningLate, "indicator-red"),
            (DemoScenario::ProblemPeriods, "indicator-green-degraded"),
            (DemoScenario::FragmentedDay, "indicator-green-degraded"),
            (
                DemoScenario::ActivityWatchUnavailable,
                "indicator-red-degraded"
            ),
        ]
    );
}

/// The status, then the headline's lines label first, as the flyout shows them.
fn shown_lines(view: &DayView, tracking: TrackingState) -> Vec<String> {
    let shown = view.shown_while(tracking);
    let headline = shown.headline;
    let status = format!("{:?} {}", shown.tone, shown.status);
    [
        Some(status.as_str()),
        headline.label.as_deref(),
        Some(&headline.value),
        headline.hint.as_deref(),
        headline.qualifier.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(spaced)
    .collect()
}

/// `MainWindow.UpdateCommands` of the .NET app: a paused working day says so and asks to resume
/// instead of forecasting, whatever the forecast would have been. A source in trouble keeps its
/// status, unlike in the .NET app, which swaps the text and keeps only the banner's colour: the
/// status bar says the tracking state instead. Resuming then brings no forecast back, so the
/// headline waits for the source instead.
#[test]
fn a_paused_working_day_asks_to_resume_unless_the_source_is_in_trouble() {
    for (scenario, in_trouble) in [
        (DemoScenario::OnTrack, false),
        (DemoScenario::Late, false),
        (DemoScenario::Complete, false),
        (DemoScenario::Morning, false),
        (DemoScenario::ProblemPeriods, false),
        (DemoScenario::Unavailable, true),
    ] {
        for (language, expected) in [
            (
                Language::French,
                if in_trouble {
                    [
                        "SourceProblem Suivi à vérifier",
                        "Votre prévision reviendra dès que la détection de l’activité sera \
                         rétablie",
                    ]
                } else {
                    [
                        "Paused En pause",
                        "Reprenez le suivi pour voir votre prévision",
                    ]
                },
            ),
            (
                Language::English,
                if in_trouble {
                    [
                        "SourceProblem Check activity tracking",
                        "Your forecast returns once activity detection works again",
                    ]
                } else {
                    ["Paused Paused", "Resume tracking to see your forecast"]
                },
            ),
        ] {
            assert_eq!(
                shown_lines(&day(scenario, language), TrackingState::Paused),
                expected,
                "{scenario:?}, in {language:?}"
            );
        }
    }
}

/// A finished working day says how far the worked row ends from its target instead of asking to
/// resume. While the source is in trouble the gap is unknown, so it points to adjusting the time
/// instead.
#[test]
fn a_finished_working_day_says_the_gap_to_its_target() {
    for (scenario, language, expected) in [
        (
            DemoScenario::Complete,
            Language::French,
            [
                "Finished Journée terminée",
                "1_h_00_min au-delà de votre objectif",
            ],
        ),
        (
            DemoScenario::Complete,
            Language::English,
            ["Finished Day finished", "1h_00m over your target"],
        ),
        (
            DemoScenario::OnTrack,
            Language::French,
            [
                "Finished Journée terminée",
                "1_h_00_min en deçà de votre objectif",
            ],
        ),
        (
            DemoScenario::OnTrack,
            Language::English,
            ["Finished Day finished", "1h_00m short of your target"],
        ),
        (
            DemoScenario::ProblemPeriods,
            Language::English,
            ["Finished Day finished", "1h_56m short of your target"],
        ),
        (
            DemoScenario::Credit,
            Language::English,
            ["Finished Day finished", "5h_48m short of your target"],
        ),
        (
            DemoScenario::Unavailable,
            Language::French,
            [
                "SourceProblem Suivi à vérifier",
                "Cliquez sur Corriger le temps pour ajouter le travail manquant",
            ],
        ),
        (
            DemoScenario::Unavailable,
            Language::English,
            [
                "SourceProblem Check activity tracking",
                "Select Adjust time to add any missing work",
            ],
        ),
    ] {
        assert_eq!(
            shown_lines(&day(scenario, language), TrackingState::Finished),
            expected,
            "{scenario:?}, in {language:?}"
        );
    }
}

/// The 6 hours worked on track meet a 6-hour target to the minute.
#[test]
fn a_finished_day_on_its_target_says_it_is_reached() {
    for (language, row, expected) in [
        (
            Language::French,
            "6_h_00_min / 6_h_00_min",
            ["Finished Journée terminée", "Objectif atteint"],
        ),
        (
            Language::English,
            "6h_00m / 6h_00m",
            ["Finished Day finished", "Target reached"],
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut host = DemoHost::open(DemoScenario::OnTrack, directory.path()).unwrap();
        let settings = Settings {
            nominal_daily_target: SignedDuration::from_hours(6),
            ..host.settings().clone()
        };
        host.save_settings(&settings).unwrap();
        let view = host.refresh(language).unwrap();
        assert_eq!(
            view.worked.as_ref().map(|worked| {
                let target = worked.target.as_ref().unwrap();
                spaced(&format!("{} / {}", worked.worked, target.duration))
            }),
            Some(row.to_owned()),
        );
        assert_eq!(
            shown_lines(&view, TrackingState::Finished),
            expected,
            "in {language:?}"
        );
    }
}

#[test]
fn a_running_day_keeps_its_forecast() {
    assert_eq!(
        shown_lines(
            &day(DemoScenario::OnTrack, Language::English),
            TrackingState::Running
        ),
        [
            "OnTrack On track",
            "Projected departure",
            "5:00_PM",
            "1h_30m to spare before your latest departure (6:30_PM)",
        ]
    );
}

/// Without a summary there is no forecast to hide: the source problem stays in front.
#[test]
fn a_paused_or_finished_day_without_a_summary_keeps_the_source_problem() {
    for view in [
        unavailable_source(BuiltInActivitySource::SOURCE_ID),
        source_in_trouble(ACTIVITY_WATCH_ID, SourceFreshness::Fresh),
    ] {
        for tracking in [TrackingState::Paused, TrackingState::Finished] {
            assert_eq!(
                shown_lines(&view, tracking),
                [
                    "SourceProblem Suivi à vérifier",
                    "Départ prévu",
                    "Prévision indisponible",
                    "Recalculée dès que le suivi reprend",
                ],
                "{tracking:?}"
            );
        }
    }
}
