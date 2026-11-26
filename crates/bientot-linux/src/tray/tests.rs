use std::sync::mpsc;
use std::thread;

use super::*;

use TrayHost::{Absent, Present};
use WatcherSignal::{Offline, Online};

fn item(label: &str, enabled: bool) -> MenuEntry<u8> {
    MenuEntry::Item {
        label: label.into(),
        command: 0,
        enabled,
    }
}

fn node(label: &str, enabled: bool, id: &str) -> MenuNode {
    MenuNode::Item {
        label: label.into(),
        enabled,
        id: id.into(),
    }
}

#[test]
fn the_dbusmenu_keeps_labels_separators_states_and_submenus_with_the_ids_of_the_generation() {
    let entries = [
        item("Ouvrir Bientôt", true),
        MenuEntry::Separator,
        MenuEntry::Submenu {
            label: "Revues".into(),
            items: vec![item("Garder", true), item("Corriger", false)],
        },
        item("Quitter", true),
    ];

    assert_eq!(
        menu(&entries, 7),
        [
            node("Ouvrir Bientôt", true, "7:0"),
            MenuNode::Separator,
            MenuNode::Submenu {
                label: "Revues".into(),
                children: vec![
                    node("Garder", true, "7:2.0"),
                    node("Corriger", false, "7:2.1")
                ],
            },
            node("Quitter", true, "7:3"),
        ]
    );
}

#[test]
fn an_underscore_in_a_label_is_shown_rather_than_read_as_the_access_key() {
    let entries = [MenuEntry::Submenu {
        label: "dossier_2026".into(),
        items: vec![item("export_diagnostic", true)],
    }];

    assert_eq!(
        menu(&entries, 1),
        [MenuNode::Submenu {
            label: "dossier__2026".into(),
            children: vec![node("export__diagnostic", true, "1:0.0")],
        }]
    );
}

#[test]
fn the_ids_of_the_dbusmenu_name_the_commands_of_the_menu_they_came_from() {
    let entries = [
        MenuEntry::Item {
            label: "Ouvrir".into(),
            command: 'o',
            enabled: true,
        },
        MenuEntry::Submenu {
            label: "Plus".into(),
            items: vec![MenuEntry::Item {
                label: "Quitter".into(),
                command: 'q',
                enabled: true,
            }],
        },
    ];
    let MenuNode::Submenu { children, .. } = &menu(&entries, 4)[1] else {
        panic!("the second entry is a submenu");
    };
    let MenuNode::Item { id, .. } = &children[0] else {
        panic!("the submenu holds an item");
    };

    assert_eq!(chosen_command(&entries, 4, id), Some('q'));
    assert_eq!(chosen_command(&entries, 5, id), None);
}

fn chosen_command(entries: &[MenuEntry<char>], generation: u64, id: &str) -> Option<char> {
    bientot_platform::tray::chosen(entries, generation, id)
}

#[test]
fn each_pixmap_turns_straight_rgba_into_argb_in_network_byte_order() {
    let one = [0x11, 0x22, 0x33, 0x44];
    let two = [
        0xff, 0x00, 0x00, 0xff, // opaque red
        0x00, 0xff, 0x00, 0x80, // half-transparent green
        0x00, 0x00, 0xff, 0x00, // transparent blue
        0x01, 0x02, 0x03, 0x04,
    ];

    assert_eq!(
        pixmaps(TrayIcon {
            pixmaps: &[(1, &one), (2, &two)],
        }),
        [
            Pixmap {
                size: 1,
                argb: vec![0x44, 0x11, 0x22, 0x33],
            },
            Pixmap {
                size: 2,
                argb: vec![
                    0xff, 0xff, 0x00, 0x00, //
                    0x80, 0x00, 0xff, 0x00, //
                    0x00, 0x00, 0x00, 0xff, //
                    0x04, 0x01, 0x02, 0x03,
                ],
            },
        ]
    );
}

#[test]
fn the_item_offers_the_sizes_of_the_panels() {
    assert_eq!(ICON_SIZES, [16, 22, 24, 32, 48]);
}

#[test]
fn a_click_at_the_origin_has_no_position_and_any_other_keeps_its_own() {
    assert_eq!(click_position(0, 0), None);
    assert_eq!(
        click_position(1909, 1064),
        Some(ScreenPoint { x: 1909, y: 1064 })
    );
    assert_eq!(click_position(0, 14), Some(ScreenPoint { x: 0, y: 14 }));
    assert_eq!(click_position(-20, 0), Some(ScreenPoint { x: -20, y: 0 }));
}

fn at(start: Instant, milliseconds: u64) -> Instant {
    start + Duration::from_millis(milliseconds)
}

#[test]
fn a_watcher_that_comes_back_within_the_grace_changes_nothing() {
    let start = Instant::now();
    let mut watch = HostWatch::new(Present, HOST_GRACE);

    assert_eq!(watch.heard(Offline, at(start, 0)), None);
    assert_eq!(watch.deadline(), Some(at(start, 5_000)));
    assert_eq!(watch.elapsed(at(start, 4_999)), None);
    assert_eq!(watch.heard(Online, at(start, 4_000)), None);
    assert_eq!(watch.deadline(), None);
    assert_eq!(watch.elapsed(at(start, 10_000)), None);
    assert_eq!(watch.told(), Present);
}

#[test]
fn a_watcher_gone_for_the_whole_grace_means_no_host_and_its_return_a_host_at_once() {
    let start = Instant::now();
    let mut watch = HostWatch::new(Present, HOST_GRACE);

    watch.heard(Offline, at(start, 0));
    assert_eq!(watch.elapsed(at(start, 5_000)), Some(Absent));
    assert_eq!(watch.told(), Absent);
    assert_eq!(watch.deadline(), None);
    assert_eq!(watch.elapsed(at(start, 60_000)), None);
    assert_eq!(watch.heard(Online, at(start, 60_000)), Some(Present));
    assert_eq!(watch.told(), Present);
}

#[test]
fn the_grace_runs_from_the_first_departure() {
    let start = Instant::now();
    let mut watch = HostWatch::new(Present, HOST_GRACE);

    watch.heard(Offline, at(start, 0));
    watch.heard(Offline, at(start, 3_000));

    assert_eq!(watch.deadline(), Some(at(start, 5_000)));
    assert_eq!(watch.elapsed(at(start, 5_000)), Some(Absent));
}

#[test]
fn a_departure_after_a_return_starts_a_new_grace() {
    let start = Instant::now();
    let mut watch = HostWatch::new(Present, HOST_GRACE);

    watch.heard(Offline, at(start, 0));
    watch.heard(Online, at(start, 1_000));
    watch.heard(Offline, at(start, 2_000));

    assert_eq!(watch.elapsed(at(start, 6_000)), None);
    assert_eq!(watch.elapsed(at(start, 7_000)), Some(Absent));
}

#[test]
fn an_item_started_without_a_watcher_tells_nothing_until_one_comes() {
    let start = Instant::now();
    let mut watch = HostWatch::new(Absent, HOST_GRACE);

    assert_eq!(watch.heard(Offline, at(start, 0)), None);
    assert_eq!(watch.deadline(), None);
    assert_eq!(watch.heard(Online, at(start, 9_000)), Some(Present));
}

/// Runs [`follow`] with `grace` on a thread; the changes it reports arrive with the time each
/// took from the start, in milliseconds.
fn followed(
    grace: Duration,
) -> (
    mpsc::Sender<WatcherSignal>,
    mpsc::Receiver<(TrayHost, u128)>,
) {
    let (signals, heard) = mpsc::channel();
    let (changes, told) = mpsc::channel();
    let start = Instant::now();
    thread::spawn(move || {
        follow(HostWatch::new(Present, grace), &heard, |host| {
            changes.send((host, start.elapsed().as_millis())).ok();
        });
    });
    (signals, told)
}

#[test]
fn following_the_watcher_tells_its_absence_once_the_grace_ran_out_and_its_return_at_once() {
    let grace = Duration::from_millis(300);
    let (signals, told) = followed(grace);

    signals.send(Offline).unwrap();
    signals.send(Online).unwrap();
    assert_eq!(told.recv_timeout(Duration::from_millis(600)).ok(), None);

    signals.send(Offline).unwrap();
    let (host, after) = told.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(host, Absent);
    assert!(after >= 600 + 300, "told after {after} ms");

    signals.send(Online).unwrap();
    let (host, _) = told.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(host, Present);

    drop(signals);
    assert_eq!(
        told.recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
}
