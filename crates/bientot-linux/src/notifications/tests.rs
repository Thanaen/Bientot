use super::*;

fn request() -> NotificationRequest {
    NotificationRequest {
        delivery_id: "2026-11-26:CompletionApproach".into(),
        title: "Bientôt".into(),
        body: "Il reste 15 minutes avant votre objectif. Départ prévu : 17:55.".into(),
    }
}

/// contracts/linux-backends.md, « Notifications ».
#[test]
fn the_server_is_sent_the_text_the_icon_the_desktop_entry_and_the_body_action() {
    let request = request();

    assert_eq!(
        notify(&request),
        Notify {
            app_name: "Bientôt",
            replaces_id: 0,
            app_icon: "dev.bientot.Bientot",
            summary: "Bientôt",
            body: "Il reste 15 minutes avant votre objectif. Départ prévu : 17:55.",
            actions: ["default", ""],
            hints: [("desktop-entry", "dev.bientot.Bientot")],
            expire_timeout: -1,
        }
    );
}

#[test]
fn the_portal_is_sent_the_delivery_id_as_the_notification_id() {
    let request = request();

    assert_eq!(
        portal_notification(&request),
        PortalNotification {
            id: "2026-11-26:CompletionApproach",
            title: "Bientôt",
            body: "Il reste 15 minutes avant votre objectif. Départ prévu : 17:55.",
            default_action: "open",
        }
    );
}

const NO_GRAPHICS: &str = "Bientôt could not open a window and has closed. Check that your \
                           system’s graphics drivers are installed, then restart the app.";

#[test]
fn the_last_resort_reaches_the_server_as_any_notification_with_its_text() {
    let request = last_resort("Bientôt", NO_GRAPHICS);

    assert_eq!(
        notify(&request),
        Notify {
            app_name: "Bientôt",
            replaces_id: 0,
            app_icon: "dev.bientot.Bientot",
            summary: "Bientôt",
            body: "Bientôt could not open a window and has closed. Check that your system’s \
                   graphics drivers are installed, then restart the app.",
            actions: ["default", ""],
            hints: [("desktop-entry", "dev.bientot.Bientot")],
            expire_timeout: -1,
        }
    );
}

#[test]
fn the_last_resort_reaches_the_portal_under_an_id_of_its_own() {
    let request = last_resort("Bientôt", NO_GRAPHICS);

    assert_eq!(
        portal_notification(&request),
        PortalNotification {
            id: "last-resort",
            title: "Bientôt",
            body: "Bientôt could not open a window and has closed. Check that your system’s \
                   graphics drivers are installed, then restart the app.",
            default_action: "open",
        }
    );
}

#[test]
fn a_click_on_the_body_gives_the_delivery_id_of_that_notification() {
    let mut delivered = Delivered::new(SERVER_BODY_ACTION);
    delivered.shown(7, "first".into());
    delivered.shown(8, "second".into());

    assert_eq!(delivered.invoked(&8, "default"), Some("second"));
    assert_eq!(delivered.invoked(&7, "default"), Some("first"));
    assert_eq!(
        delivered.invoked(&7, "default"),
        Some("first"),
        "a server that keeps the notification may send its click again"
    );
}

#[test]
fn other_applications_notifications_and_other_actions_are_no_clicks() {
    let mut delivered = Delivered::new(PORTAL_BODY_ACTION);
    delivered.shown("ours".to_owned(), "ours".into());

    assert_eq!(delivered.invoked(&"theirs".to_owned(), "open"), None);
    assert_eq!(delivered.invoked(&"ours".to_owned(), "default"), None);
    assert_eq!(delivered.invoked(&"ours".to_owned(), "open"), Some("ours"));
}

#[test]
fn a_closed_notification_is_forgotten_so_its_number_may_name_another_applications() {
    let mut delivered = Delivered::new(SERVER_BODY_ACTION);
    delivered.shown(7, "first".into());

    delivered.closed(&7);

    assert_eq!(delivered.invoked(&7, "default"), None);
}

#[test]
fn a_flatpak_shows_them_through_the_portal_and_every_other_install_through_the_server() {
    let through_the_portal: Vec<_> = InstallKind::ALL
        .into_iter()
        .filter(|kind| Service::for_install(*kind) == Service::Portal)
        .collect();
    assert_eq!(through_the_portal, [InstallKind::Flatpak]);
}
