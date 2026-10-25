//! The user-visible texts, read from a table generated from
//! `crates/bientot-app/strings/{fr-FR,en-US}.resw`. A key missing from the `.resw` files is a
//! missing [`Key`] variant: a build error.
//!
//! After editing a `.resw` file, regenerate the table with
//! `BIENTOT_UPDATE_STRINGS=1 cargo test -p bientot-app strings`; the tests fail while it lags.

use std::fmt::Display;

use bientot_core::settings::language;
use gpui::Global;

#[rustfmt::skip]
mod table;

pub use table::Key;

/// The language of the texts. `--french` and `--english` pick it, as in the .NET app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    French,
    English,
}

impl Global for Language {}

impl Language {
    pub const ALL: [Language; 2] = [Language::French, Language::English];

    /// The language tag that names its `.resw` file.
    pub fn tag(self) -> &'static str {
        match self {
            Language::French => "fr-FR",
            Language::English => "en-US",
        }
    }

    /// The language of a launch: `forced` by `--french` or `--english`, else the one saved in
    /// the settings, else the Windows display languages as `SettingsLanguageResolver` reads
    /// them. Those are not read yet, so French stands for them.
    pub fn resolve(forced: Option<Language>, saved: Option<&str>) -> Self {
        if forced == Some(Language::French) {
            return Language::French;
        }
        let tag = language::resolve(
            forced == Some(Language::English),
            saved,
            [Language::French.tag()],
        );
        Self::ALL
            .into_iter()
            .find(|language| language.tag() == tag)
            .unwrap_or_default()
    }

    pub fn text(self, key: Key) -> &'static str {
        let entry = &table::ENTRIES[key as usize];
        match self {
            Language::French => entry.fr,
            Language::English => entry.en,
        }
    }

    /// `key` with its .NET composite-format holes filled: `{0}` takes the first argument, and
    /// `{1:00}` the second, zero-padded to two digits.
    pub fn format(self, key: Key, arguments: &[&dyn Display]) -> String {
        format(self.text(key), arguments)
    }

    /// A time of day as the culture writes it: 16:54, or 4:54 PM with a no-break space, so that
    /// the time never wraps apart from its AM or PM.
    pub fn short_time(self, time: jiff::civil::Time) -> String {
        match self {
            Language::French => time.strftime("%H:%M").to_string(),
            Language::English => keep_together(&time.strftime("%-I:%M %p").to_string()),
        }
    }

    /// The abbreviated name of `weekday`, from the `;`-separated list of the `.resw` files.
    pub fn weekday_abbreviation(self, weekday: jiff::civil::Weekday) -> &'static str {
        nth(
            self.text(Key::CalendarAbbreviatedDayNames),
            weekday.to_monday_zero_offset() as usize,
        )
    }

    pub fn month_abbreviation(self, month: i8) -> &'static str {
        nth(
            self.text(Key::CalendarAbbreviatedMonthNames),
            (month - 1) as usize,
        )
    }

    pub fn weekday_name(self, weekday: jiff::civil::Weekday) -> &'static str {
        nth(
            self.text(Key::CalendarDayNames),
            weekday.to_monday_zero_offset() as usize,
        )
    }

    /// « jeudis », « Thursdays ».
    pub fn weekday_plural(self, weekday: jiff::civil::Weekday) -> &'static str {
        nth(
            self.text(Key::CalendarDayNamesPlural),
            weekday.to_monday_zero_offset() as usize,
        )
    }

    pub fn month_name(self, month: i8) -> &'static str {
        nth(self.text(Key::CalendarMonthNames), (month - 1) as usize)
    }

    /// The day in a date: French writes the first of the month « 1er », in every `d` hole.
    fn day_of_month(self, day: i8) -> String {
        match (self, day) {
            (Language::French, 1) => "1er".to_owned(),
            _ => day.to_string(),
        }
    }

    /// `key` with its holes filled by dates, written by their .NET custom format: `{0:%d}` is
    /// the day, `{1:d MMMM}` the day and the month's name, `yyyy` the year, `dddd` the
    /// weekday's name.
    pub fn format_dates(self, key: Key, dates: &[jiff::civil::Date]) -> String {
        let template = self.text(key);
        let mut output = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            let end = start + rest[start..].find('}').expect("a .resw hole is closed");
            let (index, pattern) = rest[start + 1..end]
                .split_once(':')
                .expect("a date hole names its format");
            let date = dates[index
                .parse::<usize>()
                .expect("a hole starts with its index")];
            let mut pattern = pattern;
            while !pattern.is_empty() {
                if let Some(after) = pattern.strip_prefix("dddd") {
                    output.push_str(self.weekday_name(date.weekday()));
                    pattern = after;
                } else if let Some(after) = pattern.strip_prefix("MMMM") {
                    output.push_str(self.month_name(date.month()));
                    pattern = after;
                } else if let Some(after) = pattern.strip_prefix("yyyy") {
                    output.push_str(&date.year().to_string());
                    pattern = after;
                } else if let Some(after) = pattern
                    .strip_prefix("%d")
                    .or_else(|| pattern.strip_prefix('d'))
                {
                    output.push_str(&self.day_of_month(date.day()));
                    pattern = after;
                } else {
                    let literal = pattern.chars().next().expect("the pattern is not empty");
                    // A day and its month never wrap apart.
                    output.push(if literal == ' ' { '\u{a0}' } else { literal });
                    pattern = &pattern[literal.len_utf8()..];
                }
            }
            rest = &rest[end + 1..];
        }
        output.push_str(rest);
        output
    }
}

fn nth(list: &'static str, index: usize) -> &'static str {
    list.split(';')
        .nth(index)
        .expect("the tests check the length of every name list")
}

impl Key {
    /// The key as the `.resw` files spell it, `ShellTitle.Text` for [`Key::ShellTitleText`].
    pub fn name(self) -> &'static str {
        table::ENTRIES[self as usize].name
    }
}

pub(crate) struct Entry {
    pub name: &'static str,
    pub fr: &'static str,
    pub en: &'static str,
}

/// Replaces each space with a no-break space, so that a number and its unit never wrap apart.
pub fn keep_together(text: &str) -> String {
    text.replace(' ', "\u{a0}")
}

fn format(template: &str, arguments: &[&dyn Display]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find(['{', '}']) {
        output.push_str(&rest[..start]);
        let brace = &rest[start..];
        if brace.starts_with("{{") || brace.starts_with("}}") {
            output.push_str(&brace[..1]);
            rest = &brace[2..];
            continue;
        }
        let end = brace.find('}').expect("a .resw hole is closed");
        let (index, padding) = match brace[1..end].split_once(':') {
            Some((index, padding)) => (index, Some(padding)),
            None => (&brace[1..end], None),
        };
        let index: usize = index.parse().expect("a .resw hole starts with its index");
        let value = arguments[index].to_string();
        match padding {
            Some(zeros) => {
                debug_assert!(zeros.chars().all(|digit| digit == '0'), "{template}");
                output.push_str(&"0".repeat(zeros.len().saturating_sub(value.len())));
                output.push_str(&value);
            }
            None => output.push_str(&value),
        }
        rest = &brace[end + 1..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests;
