//! The MSIX manifest against the Rust app it packages: Windows starts the executable, the startup
//! task, the toast activator and its COM server from the names in the manifest, and a mismatch
//! only shows on an installed package.

use std::path::{Path, PathBuf};

use bientot_app::launch::{Launch, Origin};
use bientot_windows::notifications::{ACTIVATOR_ARGUMENT, COM_SERVER_ARGUMENT, PACKAGED_ACTIVATOR};
use bientot_windows::startup::PACKAGED_TASK_ID;
use roxmltree::{Document, Node};

const COM: &str = "http://schemas.microsoft.com/appx/manifest/com/windows10";
const DESKTOP: &str = "http://schemas.microsoft.com/appx/manifest/desktop/windows10";

fn package_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/msix")
}

fn with_manifest(check: impl FnOnce(Node)) {
    let source = std::fs::read_to_string(package_root().join("AppxManifest.xml")).unwrap();
    let document = Document::parse(&source).unwrap();
    check(document.root_element());
}

fn element<'a, 'input>(root: Node<'a, 'input>, namespace: &str, name: &str) -> Node<'a, 'input> {
    let mut found = root
        .descendants()
        .filter(|node| node.has_tag_name((namespace, name)));
    let node = found.next().unwrap_or_else(|| panic!("no {name}"));
    assert!(found.next().is_none(), "more than one {name}");
    node
}

fn clsid(text: &str) -> u128 {
    u128::from_str_radix(&text.replace('-', ""), 16).unwrap()
}

#[test]
fn every_executable_is_the_rust_app_run_as_a_full_trust_desktop_app() {
    with_manifest(|root| {
        let executables: Vec<_> = root
            .descendants()
            .filter_map(|node| node.attribute("Executable"))
            .collect();
        assert_eq!(executables, ["worktime.exe"; 3]);
        let entry_points: Vec<_> = root
            .descendants()
            .filter_map(|node| node.attribute("EntryPoint"))
            .collect();
        assert_eq!(entry_points, ["Windows.FullTrustApplication"; 2]);
    });
}

#[test]
fn the_toast_activator_is_the_class_the_app_registers() {
    with_manifest(|root| {
        let declared = element(root, DESKTOP, "ToastNotificationActivation")
            .attribute("ToastActivatorCLSID")
            .unwrap();
        assert_eq!(clsid(declared), PACKAGED_ACTIVATOR);
        let served = element(root, COM, "Class").attribute("Id").unwrap();
        assert_eq!(clsid(served), PACKAGED_ACTIVATOR);
    });
}

#[test]
fn the_com_server_start_parses_as_a_toast_activation() {
    with_manifest(|root| {
        let arguments = element(root, COM, "ExeServer")
            .attribute("Arguments")
            .unwrap();
        let started_by_com = arguments
            .split_whitespace()
            .chain([COM_SERVER_ARGUMENT])
            .map(str::to_owned);
        assert_eq!(
            Launch::parse(started_by_com).unwrap().origin,
            Origin::ToastActivator
        );
        assert_eq!(arguments, ACTIVATOR_ARGUMENT);
    });
}

#[test]
fn the_startup_task_is_the_one_the_app_turns_on() {
    with_manifest(|root| {
        let task = element(root, DESKTOP, "StartupTask");
        assert_eq!(task.attribute("TaskId"), Some(PACKAGED_TASK_ID));
        assert_eq!(task.attribute("Enabled"), Some("false"));
    });
}

#[test]
fn the_package_needs_no_windows_app_sdk_and_carries_the_workspace_version() {
    with_manifest(|root| {
        let dependencies: Vec<_> = root
            .descendants()
            .filter(|node| node.has_tag_name("PackageDependency"))
            .filter_map(|node| node.attribute("Name"))
            .collect();
        assert_eq!(dependencies, Vec::<&str>::new());
        let identity = root
            .children()
            .find(|node| node.has_tag_name("Identity"))
            .unwrap();
        assert_eq!(identity.attribute("Name"), Some("Thanaen.WorkTime"));
        assert_eq!(
            identity.attribute("Version"),
            Some(format!("{}.0", env!("CARGO_PKG_VERSION")).as_str())
        );
    });
}

/// Whether `images` holds `file`, as itself or as a variant that `makepri` indexes under its name
/// with a `scale-*` qualifier, such as `Square44x44Logo.scale-200.png`. A `targetsize-*` variant
/// alone does not count, because the Store requires a scaled one.
fn has_scaled_image(images: &Path, file: &str) -> bool {
    if images.join(file).is_file() {
        return true;
    }
    let (stem, extension) = file.rsplit_once('.').unwrap();
    std::fs::read_dir(images).unwrap().any(|entry| {
        let name = entry.unwrap().file_name().into_string().unwrap();
        name.strip_prefix(stem)
            .and_then(|rest| rest.strip_prefix('.'))
            .and_then(|rest| rest.strip_suffix(extension))
            .and_then(|rest| rest.strip_suffix('.'))
            .is_some_and(|qualifiers| {
                qualifiers
                    .split('_')
                    .any(|qualifier| qualifier.starts_with("scale-"))
            })
    })
}

#[test]
fn every_image_the_manifest_names_ships_next_to_it() {
    with_manifest(|root| {
        let references: Vec<_> = root
            .descendants()
            .filter(Node::is_element)
            .flat_map(|node| {
                node.attributes()
                    .map(|attribute| attribute.value())
                    .chain(node.text())
                    .collect::<Vec<_>>()
            })
            .map(str::trim)
            .filter(|value| value.to_ascii_lowercase().ends_with(".png"))
            .collect();
        assert!(!references.is_empty());
        for reference in references {
            let path = package_root().join(reference.replace('\\', "/"));
            let images = path.parent().unwrap();
            let file = path.file_name().unwrap().to_str().unwrap();
            assert!(has_scaled_image(images, file), "{reference} is missing");
        }
    });
}
