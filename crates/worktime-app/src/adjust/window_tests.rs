//! What a request for « Corriger le temps… » does with the window: each request makes a form
//! of its own, with a new id and the time it was made.

use jiff::civil::date;

use super::testing::*;
use super::window::{OpenWindow, Opener, WindowRequest};
use super::*;

/// The form a request made at `now` asks for on `day` in UTC.
fn asked(day: Date, seed: Seed, id_text: &str, now: &str) -> AdjustForm {
    AdjustForm::new(
        Target {
            workday: LocalWorkday::new(day, "UTC"),
            zone: TimeZone::UTC,
        },
        seed,
        id(id_text),
        utc(now),
        Language::French,
    )
}

fn thursday(seed: Seed, id_text: &str, now: &str) -> AdjustForm {
    asked(date(2026, 10, 8), seed, id_text, now)
}

fn untouched(form: &AdjustForm) -> OpenWindow<'_> {
    OpenWindow {
        opened: form,
        shown: form,
    }
}

#[test]
fn with_no_window_a_request_opens_one_when_there_is_a_day_to_correct() {
    let blank = thursday(Seed::Blank, "first", "2026-10-08T15:00:00Z");
    assert_eq!(WindowRequest::of(None, Some(&blank)), WindowRequest::Open);
    assert_eq!(WindowRequest::of(None, None), WindowRequest::Refuse);
}

/// Ctrl+E pressed again on a blank window nobody touched, or « Modifier… » again on the same
/// correction: the window stays where it is, as it is, and shows what was asked for.
#[test]
fn an_untouched_window_asked_for_again_is_shown_as_it_is() {
    let meeting = ManualAdjustment::duration_work(
        id("meeting"),
        LocalWorkday::new(date(2026, 10, 8), "UTC"),
        SignedDuration::from_mins(30),
        utc("2026-10-08T14:00:00Z"),
    )
    .unwrap();
    let edit = || Seed::Edit(Editable::new(meeting.clone()).unwrap());
    for seed in [Seed::Blank, edit()] {
        let opened = thursday(seed.clone(), "first", "2026-10-08T15:00:00Z");
        let again = thursday(seed, "second", "2026-10-08T15:02:00Z");
        assert_eq!(
            WindowRequest::of(Some(untouched(&opened)), Some(&again)),
            WindowRequest::Show
        );
    }
}

/// An untouched window gives way to a request for another day or another form.
#[test]
fn an_untouched_window_is_replaced_by_another_form() {
    let opened = thursday(Seed::Blank, "first", "2026-10-08T15:00:00Z");
    let friday = asked(
        date(2026, 10, 9),
        Seed::Blank,
        "second",
        "2026-10-09T08:00:00Z",
    );
    let declaration = thursday(Seed::DeclareWork(None), "second", "2026-10-08T15:02:00Z");
    for other in [friday, declaration] {
        assert_eq!(
            WindowRequest::of(Some(untouched(&opened)), Some(&other)),
            WindowRequest::Open
        );
    }
}

/// A window something was typed or chosen in keeps it, whatever is asked for; so does an
/// untouched one when there is nothing to open in its place.
#[test]
fn a_changed_window_is_kept() {
    let opened = thursday(Seed::Blank, "first", "2026-10-08T15:00:00Z");
    let mut shown = opened.clone();
    shown.reason = "Réunion".into();
    let changed = OpenWindow {
        opened: &opened,
        shown: &shown,
    };
    let again = thursday(Seed::Blank, "second", "2026-10-08T15:02:00Z");
    assert_eq!(
        WindowRequest::of(Some(changed), Some(&again)),
        WindowRequest::Keep
    );
    assert_eq!(WindowRequest::of(Some(changed), None), WindowRequest::Keep);
    assert_eq!(
        WindowRequest::of(Some(untouched(&opened)), None),
        WindowRequest::Keep
    );
}

/// A save the ledger refused leaves the fields as they were asked for, and still keeps the
/// window: it shows why the save failed.
#[test]
fn a_window_whose_save_was_refused_is_kept() {
    let opened = thursday(Seed::Blank, "first", "2026-10-08T15:00:00Z");
    let mut shown = opened.clone();
    shown.submit().unwrap();
    assert_eq!(shown.finished(Err(Key::AdjustTimeErrorConflict)), None);
    let refused = OpenWindow {
        opened: &opened,
        shown: &shown,
    };
    let again = thursday(Seed::Blank, "second", "2026-10-08T15:02:00Z");
    assert_eq!(
        WindowRequest::of(Some(refused), Some(&again)),
        WindowRequest::Keep
    );
}

/// The view that asks again for what an untouched window shows is the one shown the result of
/// its save; a window kept or replaced reports to the view that opened it.
#[test]
fn only_a_window_shown_again_reports_to_the_view_that_asked() {
    let (current, asking) = (Opener::Tray, Opener::Week);
    assert_eq!(WindowRequest::Show.opener(current, asking), asking);
    for other in [
        WindowRequest::Keep,
        WindowRequest::Open,
        WindowRequest::Refuse,
    ] {
        assert_eq!(other.opener(current, asking), current);
    }
}

/// A declaration has neither the types of correction nor the classification: its window is
/// shorter. An untouched one gives way to a correction, which opens in a window of its own.
#[test]
fn a_declaration_opens_a_shorter_window_than_a_correction() {
    let meeting = ManualAdjustment::interval_work(
        id("meeting"),
        LocalWorkday::new(date(2026, 10, 8), "UTC"),
        interval("2026-10-08T09:00:00Z", "2026-10-08T10:00:00Z"),
        utc("2026-10-08T14:00:00Z"),
    )
    .unwrap();
    let blank = thursday(Seed::Blank, "first", "2026-10-08T15:00:00Z");
    let edit = thursday(
        Seed::Edit(Editable::new(meeting).unwrap()),
        "first",
        "2026-10-08T15:00:00Z",
    );
    let declaration = thursday(Seed::DeclareWork(None), "first", "2026-10-08T15:00:00Z");
    assert_eq!(
        [&blank, &edit, &declaration].map(AdjustForm::window_height),
        [680., 680., 500.]
    );
    let correction = thursday(Seed::Blank, "second", "2026-10-08T15:02:00Z");
    assert_eq!(
        WindowRequest::of(Some(untouched(&declaration)), Some(&correction)),
        WindowRequest::Open
    );
}
