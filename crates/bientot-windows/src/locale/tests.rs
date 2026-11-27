use super::names;

fn list(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn reads_each_name_of_the_list_in_order() {
    assert_eq!(names(&list("en-GB\0fr-FR\0\0")), ["en-GB", "fr-FR"]);
    assert_eq!(names(&list("fr-FR\0\0")), ["fr-FR"]);
}

#[test]
fn an_empty_list_names_no_language() {
    assert!(names(&list("\0")).is_empty());
    assert!(names(&[]).is_empty());
}
