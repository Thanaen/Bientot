use super::*;

#[test]
fn a_verbatim_drive_path_loses_its_prefix() {
    assert_eq!(
        without_verbatim_prefix(r"\\?\C:\Users\Ada\AppData\Local\WorkTime\logs"),
        r"C:\Users\Ada\AppData\Local\WorkTime\logs"
    );
}

#[test]
fn a_verbatim_share_path_becomes_a_unc_path_whatever_the_case_of_its_prefix() {
    assert_eq!(
        without_verbatim_prefix(r"\\?\UNC\server\share\logs"),
        r"\\server\share\logs"
    );
    assert_eq!(
        without_verbatim_prefix(r"\\?\unc\server\share\logs"),
        r"\\server\share\logs"
    );
}

#[test]
fn a_path_without_a_verbatim_prefix_is_kept() {
    assert_eq!(without_verbatim_prefix(r"C:\logs"), r"C:\logs");
    assert_eq!(
        without_verbatim_prefix(r"\\server\share\logs"),
        r"\\server\share\logs"
    );
    assert_eq!(without_verbatim_prefix(r"\\?"), r"\\?");
}

#[test]
fn a_directory_names_itself_whatever_the_case_and_trailing_separators() {
    assert!(names_directory(
        r"C:\Users\Ada\WorkTime\logs",
        r"c:\users\ada\worktime\LOGS\"
    ));
    assert!(names_directory(
        r"C:\Users\Élodie\logs\",
        r"C:\USERS\ÉLODIE\logs"
    ));
}

#[test]
fn a_verbatim_data_directory_names_its_final_path() {
    assert!(names_directory(
        r"C:\WorkTimeData\logs",
        r"\\?\C:\WorkTimeData\logs"
    ));
    assert!(names_directory(
        r"\\server\share\WorkTime\logs",
        r"\\?\UNC\server\share\WorkTime\logs"
    ));
    assert!(is_directly_inside(
        r"C:\WorkTimeData\logs\worktime-20260831.jsonl",
        r"\\?\C:\WorkTimeData\logs"
    ));
    assert!(!names_directory(
        r"D:\elsewhere\logs",
        r"\\?\C:\WorkTimeData\logs"
    ));
}

#[test]
fn a_directory_reached_through_a_link_does_not_name_itself() {
    assert!(!names_directory(
        r"D:\elsewhere\logs",
        r"C:\Users\Ada\WorkTime\logs"
    ));
    assert!(!names_directory(
        r"C:\Users\Ada\WorkTime\logs2",
        r"C:\Users\Ada\WorkTime\logs"
    ));
}

#[test]
fn a_file_is_inside_the_directory_that_directly_holds_it() {
    let directory = r"C:\Users\Ada\WorkTime\logs";
    assert!(is_directly_inside(
        r"C:\USERS\ADA\WorkTime\logs\worktime-20261008-080000-0000000-0123.jsonl",
        directory
    ));
    assert!(is_directly_inside(r"C:\worktime.jsonl", r"C:\"));
}

#[test]
fn a_file_elsewhere_or_deeper_is_not_inside_the_directory() {
    let directory = r"C:\Users\Ada\WorkTime\logs";
    assert!(!is_directly_inside(
        r"D:\elsewhere\worktime.jsonl",
        directory
    ));
    assert!(!is_directly_inside(
        r"C:\Users\Ada\WorkTime\logs\nested\worktime.jsonl",
        directory
    ));
    assert!(!is_directly_inside(
        r"C:\Users\Ada\WorkTime\worktime.jsonl",
        directory
    ));
    assert!(!is_directly_inside("worktime.jsonl", directory));
}
