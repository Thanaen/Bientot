use std::path::{Path, PathBuf};

use super::*;

#[test]
fn the_files_go_in_the_runtime_folder_else_in_the_data_folder() {
    let data = Path::new("/home/ana/.local/share/Bientot");
    assert_eq!(
        instance_folder(Some("/run/user/1000".into()), data),
        PathBuf::from("/run/user/1000/bientot")
    );
    for unusable in [None, Some("".into()), Some("run/user/1000".into())] {
        assert_eq!(
            instance_folder(unusable, data),
            PathBuf::from("/home/ana/.local/share/Bientot/run")
        );
    }
}

#[test]
fn the_socket_of_the_users_folder_fits_sun_path_without_a_runtime_folder() {
    let key = format!("WorkTime-{}", "0123456789abcdef".repeat(4));
    let stem = file_stem(&key);
    assert_eq!(stem, format!("WorkTime-{}", "0123456789abcdef".repeat(2)));
    let socket = instance_folder(None, Path::new("/home/someone.long/.local/share/Bientot"))
        .join(format!("{stem}.sock"));
    assert!(socket.as_os_str().len() < 108, "{}", socket.display());
    assert_eq!(file_stem("short"), "short");
}
