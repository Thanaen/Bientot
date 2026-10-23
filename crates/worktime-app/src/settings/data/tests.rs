use jiff::civil::date;

use super::*;

fn here() -> Option<DataDirectory> {
    Some(DataDirectory::from(PathBuf::from(
        "C:/Users/Camille/AppData/Local/WorkTime",
    )))
}

fn chosen(days: Option<(Date, Date)>, this_pc: Option<(Date, Date)>) -> Chosen {
    Chosen {
        file: PathBuf::from("WorkTime-2026-10-21.worktime"),
        contents: Contents {
            exported_at: "2026-10-21T16:04:00Z".parse().unwrap(),
            days,
        },
        exported_on: date(2026, 10, 21),
        this_pc,
    }
}

fn confirming(page: &mut DataPage, days: Option<(Date, Date)>, this_pc: Option<(Date, Date)>) {
    assert!(page.start(Action::Import).is_some());
    page.read(Some(Ok(chosen(days, this_pc))));
}

fn row(page: &DataPage, which: Row) -> ActionRow {
    page.view()
        .rows
        .into_iter()
        .find(|row| row.row == which)
        .unwrap()
}

#[test]
fn both_actions_wait_for_the_one_running() {
    let mut page = DataPage::new(Language::French, here(), None);
    assert!(page.view().rows.iter().all(|row| row.enabled));

    assert!(page.start(Action::Export).is_some());

    assert!(page.start(Action::Import).is_none());
    let view = page.view();
    assert!(view.rows.iter().all(|row| !row.enabled));
    assert_eq!(view.status, Some("Exportation…"));
    assert!(view.busy);
}

#[test]
fn an_export_names_its_file_and_shows_it_until_the_next_action() {
    let mut page = DataPage::new(Language::French, here(), None);
    page.start(Action::Export);
    let path = PathBuf::from("D:/WorkTime-2026-10-22.worktime");

    page.exported(Some(Exported::Saved(path.clone())));

    let export = row(&page, Row::Export);
    assert_eq!(
        export.status.as_deref(),
        Some("Données exportées · WorkTime-2026-10-22.worktime. Importez-le sur votre nouveau PC.")
    );
    assert_eq!(export.reveal, Some("Afficher dans l’Explorateur"));
    assert_eq!(page.revealed(Row::Export), Some(path.as_path()));
    assert!(!page.view().busy);
    page.start(Action::Import);
    page.read(None);
    assert_eq!(row(&page, Row::Export).status, None);
    assert_eq!(page.revealed(Row::Export), None);
}

#[test]
fn a_cancelled_save_dialog_says_nothing() {
    let mut page = DataPage::new(Language::French, here(), None);
    page.start(Action::Export);

    page.exported(None);

    assert!(
        page.view()
            .rows
            .iter()
            .all(|row| row.status.is_none() && row.error.is_none())
    );
    assert!(page.ready());
}

#[test]
fn a_failed_export_asks_for_another_place_on_its_row() {
    let mut page = DataPage::new(Language::English, here(), None);
    page.start(Action::Export);

    page.exported(Some(Exported::Failed));

    assert_eq!(
        row(&page, Row::Export).error,
        Some("WorkTime could not save the file. Choose another location and try again.")
    );
    assert_eq!(row(&page, Row::Import).error, None);
}

#[test]
fn each_refusal_says_why_on_the_import_row() {
    for (refusal, error) in [
        (
            Refusal::NotATransfer,
            "Ce fichier n’est pas une exportation WorkTime. Choisissez un fichier .worktime.",
        ),
        (
            Refusal::NewerVersion,
            "Ce fichier vient d’une version plus récente de WorkTime. Mettez WorkTime à jour sur ce PC, puis réessayez.",
        ),
        (
            Refusal::Unreadable,
            "WorkTime n’a pas pu lire ce fichier. Copiez-le sur ce PC, puis réessayez.",
        ),
    ] {
        let mut page = DataPage::new(Language::French, here(), None);
        page.start(Action::Import);
        page.read(Some(Err(refusal)));
        assert_eq!(row(&page, Row::Import).error, Some(error));
        assert_eq!(page.dialog(), None);
    }
}

#[test]
fn the_dialog_tells_both_periods() {
    let mut page = DataPage::new(Language::French, here(), None);
    confirming(
        &mut page,
        Some((date(2026, 3, 2), date(2026, 10, 21))),
        Some((date(2026, 10, 1), date(2026, 10, 21))),
    );

    let dialog = page.dialog().unwrap();

    assert_eq!(
        dialog.paragraphs,
        [
            "Ce fichier, exporté le 21\u{a0}octobre\u{a0}2026, contient votre historique du \
             2\u{a0}mars\u{a0}2026 au 21\u{a0}octobre\u{a0}2026.",
            "Ce PC contient votre historique du 1er\u{a0}octobre\u{a0}2026 au \
             21\u{a0}octobre\u{a0}2026."
        ]
    );
    assert_eq!(dialog.loss, None);
    assert_eq!(dialog.title, "Remplacer les données de ce PC\u{202f}?");
    assert_eq!(dialog.confirm, "Remplacer et redémarrer");
    assert!(!page.ready());
}

#[test]
fn the_dialog_warns_of_the_days_this_pc_would_lose() {
    let mut page = DataPage::new(Language::English, here(), None);
    confirming(
        &mut page,
        Some((date(2026, 3, 2), date(2026, 10, 1))),
        Some((date(2026, 3, 2), date(2026, 10, 21))),
    );
    assert_eq!(
        page.dialog().unwrap().loss.as_deref(),
        Some(
            "Days recorded on this PC after October\u{a0}1,\u{a0}2026, will no longer be in the history."
        )
    );

    page.cancel();
    confirming(
        &mut page,
        Some((date(2026, 6, 1), date(2026, 10, 21))),
        Some((date(2026, 3, 2), date(2026, 10, 21))),
    );
    assert_eq!(
        page.dialog().unwrap().loss.as_deref(),
        Some(
            "Days recorded on this PC outside the file’s period will no longer be in the history."
        )
    );

    page.cancel();
    confirming(
        &mut page,
        None,
        Some((date(2026, 3, 2), date(2026, 10, 21))),
    );
    let dialog = page.dialog().unwrap();
    assert_eq!(
        dialog.paragraphs[0],
        "This file, exported on October\u{a0}21,\u{a0}2026, holds no history."
    );
    assert_eq!(
        dialog.loss.as_deref(),
        Some("This PC will have no history left.")
    );
}

#[test]
fn a_new_pc_has_nothing_to_lose() {
    let mut page = DataPage::new(Language::French, here(), None);
    confirming(
        &mut page,
        Some((date(2026, 3, 2), date(2026, 10, 21))),
        None,
    );

    let dialog = page.dialog().unwrap();

    assert_eq!(dialog.paragraphs.len(), 1);
    assert_eq!(dialog.loss, None);
}

#[test]
fn cancelling_changes_nothing() {
    let mut page = DataPage::new(Language::French, here(), None);
    confirming(&mut page, None, None);

    page.cancel();

    assert_eq!(page.dialog(), None);
    assert!(page.ready());
    assert_eq!(page.confirm(), None);
}

#[test]
fn confirming_hands_the_file_over_and_waits_for_the_restart() {
    let mut page = DataPage::new(Language::French, here(), None);
    confirming(&mut page, None, None);

    assert_eq!(
        page.confirm(),
        Some((
            PathBuf::from("WorkTime-2026-10-21.worktime"),
            here().unwrap()
        ))
    );

    assert_eq!(page.dialog(), None);
    assert!(!page.ready());
    assert_eq!(page.view().status, Some("Redémarrage de WorkTime…"));
    page.import_failed();
    assert_eq!(
        row(&page, Row::Import).error,
        Some("WorkTime n’a pas pu importer ce fichier. Vos données n’ont pas changé. Réessayez.")
    );
    assert!(page.ready());
}

fn backup() -> Backup {
    Backup {
        file: PathBuf::from("backups/before-import-2026-10-22-11-30-00.worktime"),
        saved_on: date(2026, 10, 22),
    }
}

#[test]
fn the_copy_of_the_last_import_is_offered_to_restore_and_in_file_explorer() {
    let page = DataPage::new(Language::French, here(), Some(backup()));

    let shown = row(&page, Row::Backup);

    assert_eq!(shown.title, "Copie d’avant l’import");
    assert_eq!(
        shown.description,
        "Enregistrée le 22\u{a0}octobre\u{a0}2026. Restaurez-la pour revenir aux données remplacées."
    );
    assert_eq!(shown.label, "Restaurer…");
    assert_eq!(shown.action, Action::Restore);
    assert!(shown.enabled);
    assert_eq!(shown.reveal, Some("Afficher dans l’Explorateur"));
    assert_eq!(page.revealed(Row::Backup), Some(backup().file.as_path()));
    assert_eq!(
        DataPage::new(Language::French, here(), None)
            .view()
            .rows
            .len(),
        2
    );
}

#[test]
fn restoring_the_copy_asks_like_an_import_and_stages_the_copy() {
    let mut page = DataPage::new(Language::French, here(), Some(backup()));

    assert_eq!(page.start(Action::Restore), here());
    let view = page.view();
    assert_eq!(view.status, Some("Lecture de la copie…"));
    assert!(view.rows.iter().all(|row| !row.enabled));

    page.read(Some(Ok(Chosen {
        file: backup().file,
        ..chosen(
            Some((date(2026, 9, 1), date(2026, 10, 22))),
            Some((date(2026, 9, 1), date(2026, 10, 23))),
        )
    })));

    let dialog = page.dialog().unwrap();
    assert_eq!(dialog.title, "Remplacer les données de ce PC\u{202f}?");
    assert_eq!(
        dialog.paragraphs[0],
        "Cette copie, enregistrée le 21\u{a0}octobre\u{a0}2026, contient votre historique du \
         1er\u{a0}septembre\u{a0}2026 au 22\u{a0}octobre\u{a0}2026."
    );
    assert_eq!(dialog.confirm, "Remplacer et redémarrer");
    assert_eq!(page.confirm(), Some((backup().file, here().unwrap())));

    page.import_failed();
    assert_eq!(
        row(&page, Row::Backup).error,
        Some("WorkTime n’a pas pu restaurer cette copie. Vos données n’ont pas changé. Réessayez.")
    );
    assert_eq!(row(&page, Row::Import).error, None);
}

#[test]
fn a_copy_that_no_longer_reads_says_so_on_its_row() {
    let mut page = DataPage::new(Language::French, here(), Some(backup()));
    page.start(Action::Restore);

    page.read(Some(Err(Refusal::Unreadable)));

    assert_eq!(
        row(&page, Row::Backup).error,
        Some(
            "WorkTime n’a pas pu lire cette copie. Affichez-la dans l’Explorateur, puis importez-la \
             depuis «\u{a0}Importer…\u{a0}»."
        )
    );
    assert_eq!(row(&page, Row::Import).error, None);
    assert!(page.ready());
}

#[test]
fn without_a_copy_nothing_restores() {
    let mut page = DataPage::new(Language::French, here(), None);

    assert_eq!(page.start(Action::Restore), None);
    assert!(page.ready());
}

#[test]
fn a_demo_offers_neither_action() {
    let mut page = DataPage::new(Language::French, None, None);

    assert_eq!(page.start(Action::Export), None);

    let view = page.view();
    assert!(
        view.rows
            .iter()
            .all(|row| !row.enabled && row.tone == RowTone::Unavailable)
    );
    assert_eq!(
        view.status,
        Some("L’export et l’import sont désactivés dans la démo.")
    );
}
