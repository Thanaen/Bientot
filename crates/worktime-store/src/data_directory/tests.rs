//! `AppDataDirectoryTests`, and the .NET casing and keys from `tests/fixtures/data-directory.json`.

use serde_json::Value;

use super::*;

const THROWAWAY_ID: u128 = 0x0f8fad5b_d9cb_469f_a165_70867728950e;

#[test]
fn the_folders_sit_where_the_dotnet_app_puts_them() {
    let explicit = std::env::temp_dir().join("WorkTime Data");
    assert_eq!(DataDirectory::named(&explicit).unwrap().path(), explicit);
    assert_eq!(
        DataDirectory::throwaway(Path::new("/tmp"), THROWAWAY_ID).path(),
        Path::new("/tmp/BientotDemo/0f8fad5bd9cb469fa16570867728950e")
    );
    assert_eq!(
        DataDirectory::user(Path::new("/home/ana/.local/share")).path(),
        Path::new("/home/ana/.local/share/Bientot")
    );
}

#[test]
fn a_relative_data_dir_resolves_against_the_current_directory() {
    assert_eq!(
        DataDirectory::named(Path::new("portable")).unwrap().path(),
        std::env::current_dir().unwrap().join("portable")
    );
    assert!(DataDirectory::named(Path::new("")).is_err());
}

#[cfg(windows)]
#[test]
fn the_folders_sit_where_the_dotnet_app_puts_them_on_windows() {
    let named = |path| DataDirectory::named(Path::new(path)).unwrap();
    assert_eq!(
        named(r"D:\WorkTime Data\").path().as_os_str(),
        r"D:\WorkTime Data\"
    );
    assert_eq!(
        named(r"D:\a\..\WorkTime").path().as_os_str(),
        r"D:\WorkTime"
    );
    assert_eq!(
        DataDirectory::throwaway(Path::new(r"C:\Users\Ana\AppData\Local\Temp"), THROWAWAY_ID)
            .path()
            .as_os_str(),
        r"C:\Users\Ana\AppData\Local\Temp\BientotDemo\0f8fad5bd9cb469fa16570867728950e"
    );
    assert_eq!(
        DataDirectory::user(Path::new(r"C:\Users\Ana\AppData\Local"))
            .path()
            .as_os_str(),
        r"C:\Users\Ana\AppData\Local\Bientot"
    );
}

#[test]
fn the_same_folder_in_any_case_gives_the_same_key() {
    for (folder, expected) in [
        (
            r"C:\Users\Ana\AppData\Local\WorkTime",
            "WorkTime-b87383584653d36e172bb9f916813c99af6c1b873c5c84681d930cffdac58692",
        ),
        (
            r"c:\users\ana\appdata\local\worktime\",
            "WorkTime-b87383584653d36e172bb9f916813c99af6c1b873c5c84681d930cffdac58692",
        ),
        (
            r"C:\USERS\ANA\APPDATA\LOCAL\WORKTIME\\",
            "WorkTime-b87383584653d36e172bb9f916813c99af6c1b873c5c84681d930cffdac58692",
        ),
        (
            r"D:\WorkTime Data",
            "WorkTime-e790f8c42a78c8ca5d9151e9a4abb9520f0b79680330f520a1c3bc3c47532fbf",
        ),
    ] {
        assert_eq!(
            DataDirectory::from(PathBuf::from(folder)).instance_key(),
            expected,
            "{folder}"
        );
    }
}

#[test]
fn files_sit_where_the_dotnet_app_puts_them() {
    let directory = DataDirectory::from(PathBuf::from("/data"));

    assert_eq!(
        directory.settings_file(),
        Path::new("/data").join("settings.json")
    );
    assert_eq!(
        directory.database_file(),
        Path::new("/data").join("worktime.db")
    );
    assert_eq!(
        directory.secrets_directory(),
        Path::new("/data").join("secrets")
    );
    assert_eq!(
        directory.app_state_file(),
        Path::new("/data").join("app-state.json")
    );
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/data-directory.json")).unwrap()
}

/// The fixture lists every Unicode scalar value that .NET's `ToUpperInvariant` changes. .NET 10
/// casing follows Unicode 16, and Rust's follows Unicode 17, which added these lowercase letters:
/// three Latin letters and the Beria Erfe script. .NET leaves them as they are.
const UNICODE_17_LOWERCASE: [std::ops::RangeInclusive<u32>; 4] = [
    0xA7CF..=0xA7CF,
    0xA7D3..=0xA7D3,
    0xA7D5..=0xA7D5,
    0x16EBB..=0x16ED3,
];

#[test]
fn upper_case_matches_dotnet_for_every_character() {
    let fixture = fixture();
    let upper = fixture["upper"].as_object().unwrap();
    let mismatches: Vec<u32> = (0..=0x10FFFF)
        .filter_map(char::from_u32)
        .filter(|&character| {
            let expected = upper
                .get(&format!("{:04X}", u32::from(character)))
                .map_or_else(
                    || character.to_string(),
                    |upper| upper.as_str().unwrap().to_owned(),
                );
            upper_invariant(&character.to_string()) != expected
        })
        .map(u32::from)
        .collect();
    let unicode_17: Vec<u32> = UNICODE_17_LOWERCASE.into_iter().flatten().collect();

    assert_eq!(mismatches, unicode_17);
}

#[test]
fn instance_keys_match_dotnet() {
    let fixture = fixture();
    let cases = fixture["instanceKeys"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        assert_eq!(
            DataDirectory::from(PathBuf::from(case["dataDirectory"].as_str().unwrap()))
                .instance_key(),
            case["key"],
            "{}",
            case["dataDirectory"]
        );
    }
}
