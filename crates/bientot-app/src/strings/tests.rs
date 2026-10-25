//! The generated table against the `.resw` files, and the catalogue's rules: the same keys
//! and holes in both languages, no empty value, no old app name, typographic apostrophes and
//! French spacing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::*;

const UPDATE_VARIABLE: &str = "BIENTOT_UPDATE_STRINGS";

fn resw_path(language: Language) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("strings")
        .join(format!("{}.resw", language.tag()))
}

fn table_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/strings/table.rs")
}

/// Every key of one `.resw` file and its value, failing on a key defined twice.
fn read_resw(language: Language) -> BTreeMap<String, String> {
    let path = resw_path(language);
    let source = std::fs::read_to_string(&path).unwrap();
    let document = roxmltree::Document::parse(&source).unwrap();
    let mut entries = BTreeMap::new();
    for data in document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("data"))
    {
        let name = data.attribute("name").unwrap().to_owned();
        let value = data
            .children()
            .find(|node| node.has_tag_name("value"))
            .and_then(|value| value.text())
            .unwrap_or_default()
            .to_owned();
        assert!(
            entries.insert(name.clone(), value).is_none(),
            "{} defines {name} twice",
            language.tag()
        );
    }
    entries
}

/// The variant of a `.resw` key: the key without its dots, `ShellTitle.Text` → `ShellTitleText`.
fn variant(name: &str) -> String {
    assert!(
        name.starts_with(|first: char| first.is_ascii_uppercase())
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '.'),
        "{name} cannot become a Rust identifier"
    );
    name.replace('.', "")
}

fn generate() -> String {
    let french = read_resw(Language::French);
    let english = read_resw(Language::English);
    let french_keys: BTreeSet<_> = french.keys().collect();
    let english_keys: BTreeSet<_> = english.keys().collect();
    assert_eq!(
        french_keys.symmetric_difference(&english_keys).count(),
        0,
        "the two .resw files must define the same keys: {:?}",
        french_keys
            .symmetric_difference(&english_keys)
            .collect::<Vec<_>>()
    );

    let mut variants = BTreeSet::new();
    let mut enumeration = String::new();
    let mut entries = String::new();
    for (name, fr) in &french {
        let variant = variant(name);
        assert!(variants.insert(variant.clone()), "two keys give {variant}");
        enumeration.push_str(&format!("    {variant},\n"));
        entries.push_str(&format!(
            "    Entry {{ name: {name:?}, fr: {fr:?}, en: {:?} }},\n",
            english[name]
        ));
    }
    format!(
        "// Generated from crates/bientot-app/strings/{{fr-FR,en-US}}.resw by\n\
         // `{UPDATE_VARIABLE}=1 cargo test -p bientot-app strings`. Do not edit.\n\
         \n\
         use super::Entry;\n\
         \n\
         /// One key of the `.resw` files, named after it without its dots.\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n\
         pub enum Key {{\n{enumeration}}}\n\
         \n\
         pub(super) static ENTRIES: [Entry; {}] = [\n{entries}];\n",
        french.len()
    )
}

#[test]
fn the_table_matches_the_resw_files() {
    let generated = generate();
    if std::env::var_os(UPDATE_VARIABLE).is_some() {
        std::fs::write(table_path(), &generated).unwrap();
        return;
    }
    let checked_in = std::fs::read_to_string(table_path()).unwrap();
    assert!(
        checked_in == generated,
        "src/strings/table.rs is out of date with the .resw files: run \
         `{UPDATE_VARIABLE}=1 cargo test -p bientot-app strings` and commit the result"
    );
}

#[test]
fn every_key_reads_its_value_in_each_language() {
    let french = read_resw(Language::French);
    let english = read_resw(Language::English);
    for entry in &table::ENTRIES {
        assert_eq!(entry.fr, french[entry.name], "{}", entry.name);
        assert_eq!(entry.en, english[entry.name], "{}", entry.name);
    }
    assert_eq!(
        Language::French.text(Key::WorkdayStatusDegraded),
        "Suivi à vérifier"
    );
    assert_eq!(
        Language::English.text(Key::WorkdayStatusDegraded),
        "Check activity tracking"
    );
}

/// The holes `{0}`, `{1:00}` of a value, by index.
fn holes(value: &str) -> BTreeSet<usize> {
    value
        .match_indices('{')
        .filter(|(start, _)| !value[*start + 1..].starts_with('{'))
        .filter_map(|(start, _)| {
            let digits: String = value[start + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .collect()
}

#[test]
fn both_languages_fill_the_same_holes() {
    let mismatched: Vec<_> = table::ENTRIES
        .iter()
        .filter(|entry| holes(entry.fr) != holes(entry.en))
        .map(|entry| entry.name)
        .collect();
    assert_eq!(mismatched, Vec::<&str>::new());
}

#[test]
fn no_value_is_empty() {
    let mut empty = Vec::new();
    for entry in &table::ENTRIES {
        for (tag, value) in [("fr-FR", entry.fr), ("en-US", entry.en)] {
            if value.trim().is_empty() {
                empty.push(format!("{tag}: {}", entry.name));
            }
        }
    }
    assert_eq!(empty, Vec::<String>::new());
}

/// The app is Bientôt: the old name may appear only as the `.worktime` extension of the files
/// it still imports.
#[test]
fn no_value_names_the_app_worktime() {
    let mut faults = Vec::new();
    for entry in &table::ENTRIES {
        for (tag, value) in [("fr-FR", entry.fr), ("en-US", entry.en)] {
            if value
                .to_lowercase()
                .replace(".worktime", "")
                .contains("worktime")
            {
                faults.push(format!("{tag}: {}", entry.name));
            }
        }
    }
    assert_eq!(faults, Vec::<String>::new());
}

/// The French spacing of `value` that breaks the catalogue's rule: a no-break space
/// (U+00A0) before `:` and `»` and after `«`, a narrow no-break space (U+202F) before `;`,
/// `?` and `!`. A `:` or `;` with no space after it, as in `H:mm` or a name list, is not
/// punctuation.
fn french_spacing_faults(value: &str) -> Vec<String> {
    let characters: Vec<char> = value.chars().collect();
    let mut faults = Vec::new();
    for (index, &character) in characters.iter().enumerate() {
        let before = index.checked_sub(1).map(|before| characters[before]);
        let after = characters.get(index + 1).copied();
        let (neighbour, space) = match character {
            '«' => (after, '\u{a0}'),
            '»' => (before, '\u{a0}'),
            ':' | ';' if after.is_some_and(|after| !after.is_whitespace()) => continue,
            ':' => (before, '\u{a0}'),
            ';' | '?' | '!' => (before, '\u{202f}'),
            _ => continue,
        };
        if neighbour != Some(space) {
            faults.push(format!("{character} wants U+{:04X}", space as u32));
        }
    }
    faults
}

#[test]
fn values_use_typographic_apostrophes_and_french_spacing() {
    let mut faults = Vec::new();
    for entry in &table::ENTRIES {
        for (tag, value) in [("fr-FR", entry.fr), ("en-US", entry.en)] {
            if value.contains('\'') {
                faults.push(format!("{tag}: {} uses ' instead of ’", entry.name));
            }
        }
        for fault in french_spacing_faults(entry.fr) {
            faults.push(format!("fr-FR: {}: {fault}", entry.name));
        }
    }
    assert_eq!(faults, Vec::<String>::new());
    assert_eq!(
        french_spacing_faults("Clé : « a » ; b ? c !"),
        [
            ": wants U+00A0",
            "« wants U+00A0",
            "» wants U+00A0",
            "; wants U+202F",
            "? wants U+202F",
            "! wants U+202F",
        ]
    );
    assert_eq!(
        french_spacing_faults("Durée\u{a0}: 7:00 ; lun.;mar. «\u{a0}x\u{a0}» ?"),
        ["; wants U+202F", "? wants U+202F"]
    );
}

#[test]
fn name_lists_have_one_name_per_day_and_month() {
    for language in Language::ALL {
        for (key, count) in [
            (Key::CalendarAbbreviatedDayNames, 7),
            (Key::CalendarAbbreviatedMonthNames, 12),
            (Key::CalendarDayNames, 7),
            (Key::CalendarMonthNames, 12),
        ] {
            let names: Vec<_> = language.text(key).split(';').collect();
            assert_eq!(names.len(), count, "{key:?} in {}", language.tag());
            assert!(names.iter().all(|name| !name.trim().is_empty()));
        }
    }
    assert_eq!(
        Language::French.weekday_abbreviation(jiff::civil::Weekday::Thursday),
        "Jeu."
    );
    assert_eq!(Language::English.month_abbreviation(10), "Oct");
}

#[test]
fn fills_holes_as_dotnet_composite_formatting_does() {
    assert_eq!(
        Language::French.format(Key::WorkdayDurationFormat, &[&7, &4]),
        "7 h 04 min"
    );
    assert_eq!(
        Language::English.format(Key::WorkdayDurationFormat, &[&12, &30]),
        "12h 30m"
    );
    assert_eq!(
        Language::English.format(Key::WorkdayTooltipFormat, &[&"a", &"b", &"c", &"d"]),
        "a · b/c · d"
    );
    assert_eq!(format("{{0}} {0}", &[&1]), "{0} 1");
}

#[test]
fn writes_times_as_each_culture_does() {
    let time = jiff::civil::time(16, 54, 0, 0);
    assert_eq!(Language::French.short_time(time), "16:54");
    assert_eq!(Language::English.short_time(time), "4:54\u{a0}PM");
    assert_eq!(
        Language::English.short_time(jiff::civil::time(9, 5, 0, 0)),
        "9:05\u{a0}AM"
    );
}

#[test]
fn a_forced_language_beats_the_saved_one_and_french_is_the_default() {
    assert_eq!(
        Language::resolve(Some(Language::French), Some("en-US")),
        Language::French
    );
    assert_eq!(Language::resolve(None, Some("en-US")), Language::English);
    assert_eq!(Language::resolve(None, Some("de-DE")), Language::French);
    assert_eq!(Language::resolve(None, None), Language::French);
}

#[test]
fn writes_week_titles_with_the_dotnet_date_patterns() {
    let (monday, friday) = (
        jiff::civil::date(2026, 8, 31),
        jiff::civil::date(2026, 9, 4),
    );
    assert_eq!(
        Language::French.format_dates(Key::HistoryWeekTitle, &[monday, friday]),
        "Semaine du 31\u{a0}août au 4\u{a0}septembre"
    );
    assert_eq!(
        Language::English.format_dates(Key::HistoryWeekTitle, &[monday, friday]),
        "Week of August\u{a0}31 – September\u{a0}4"
    );
    let (monday, friday) = (
        jiff::civil::date(2026, 10, 5),
        jiff::civil::date(2026, 10, 9),
    );
    assert_eq!(
        Language::French.format_dates(Key::HistoryWeekTitleSameMonth, &[monday, friday]),
        "Semaine du 5 au 9\u{a0}octobre"
    );
    assert_eq!(
        Language::English.format_dates(Key::HistoryWeekTitleSameMonth, &[monday, friday]),
        "Week of October\u{a0}5–9"
    );
}

#[test]
fn writes_a_release_date_with_its_year_and_the_french_first_of_the_month() {
    let release = jiff::civil::date(2026, 10, 21);
    assert_eq!(
        Language::French.format_dates(Key::SettingsVersionReleasedFormat, &[release]),
        "Publiée le 21\u{a0}octobre\u{a0}2026"
    );
    assert_eq!(
        Language::English.format_dates(Key::SettingsVersionReleasedFormat, &[release]),
        "Released October\u{a0}21,\u{a0}2026"
    );
    let first = jiff::civil::date(2026, 10, 1);
    assert_eq!(
        Language::French.format_dates(Key::SettingsVersionReleasedFormat, &[first]),
        "Publiée le 1er\u{a0}octobre\u{a0}2026"
    );
    assert_eq!(
        Language::French.format_dates(
            Key::HistoryWeekTitleSameMonth,
            &[first, jiff::civil::date(2026, 10, 2)]
        ),
        "Semaine du 1er au 2\u{a0}octobre"
    );
}
