//! The read-only view of `settings.json`: its lines, coloured as an editor colours JSON, cut
//! into the rows of a box of a given width, and the rows a box of a given height shows.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Plain,
    /// A member's name.
    Key,
    String,
    /// A number, `true`, `false` or `null`.
    Literal,
}

/// The lines of a file's text, without its byte order mark nor its line breaks.
pub fn lines(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

/// `line` cut into the parts an editor colours. A string followed by a colon is a key.
pub fn tokens(line: &str) -> Vec<(Token, &str)> {
    let mut parts: Vec<(Token, &str)> = Vec::new();
    let mut plain_start = 0;
    let mut index = 0;
    let bytes = line.as_bytes();
    while index < bytes.len() {
        let start = index;
        let token = match bytes[index] {
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index = (index + 1).min(bytes.len());
                let after = line[index..].trim_start();
                if after.starts_with(':') {
                    Token::Key
                } else {
                    Token::String
                }
            }
            byte if byte == b'-' || byte.is_ascii_alphanumeric() => {
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || b"+-.".contains(&bytes[index]))
                {
                    index += 1;
                }
                Token::Literal
            }
            _ => {
                index += 1;
                continue;
            }
        };
        if plain_start < start {
            parts.push((Token::Plain, &line[plain_start..start]));
        }
        parts.push((token, &line[start..index]));
        plain_start = index;
    }
    if plain_start < line.len() {
        parts.push((Token::Plain, &line[plain_start..]));
    }
    parts
}

/// The parts of `line` within `range`, coloured as in the whole line: a string cut by the end
/// of a row keeps its colour on the next one.
pub fn tokens_within(line: &str, range: Range<usize>) -> Vec<(Token, &str)> {
    let mut start = 0;
    tokens(line)
        .into_iter()
        .filter_map(|(token, part)| {
            let (part_start, part_end) = (start, start + part.len());
            start = part_end;
            let (from, to) = (part_start.max(range.start), part_end.min(range.end));
            (from < to).then(|| (token, &line[from..to]))
        })
        .collect()
}

/// One row of the box: the bytes `range` of the line at `line`, after `indent` blank columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub range: Range<usize>,
    /// The hanging indent of a row that goes on with its line; 0 on the line's first row.
    pub indent: usize,
}

impl Row {
    /// The row that starts its line, which carries the line's number.
    pub fn starts_line(&self) -> bool {
        self.range.start == 0
    }
}

/// `lines` in rows of at most `columns` characters, indent included: the font is monospaced,
/// so the box's width is a count of characters. A longer line goes on in the rows under it,
/// indented two columns past its own indentation. A row ends after a space or a comma outside
/// a string, else where a string, number or name starts; only a part longer than a whole row
/// is cut where the row ends. An empty line is one empty row.
pub fn rows(lines: &[String], columns: usize) -> Vec<Row> {
    let columns = columns.max(1);
    let mut rows = Vec::new();
    for (index, text) in lines.iter().enumerate() {
        let lead = text.chars().take_while(|c| c.is_whitespace()).count();
        // Never more than half the row, so that a narrow box still shows the text.
        let hang = (lead + 2).min(columns / 2);
        let breaks = breaks(text, lead);
        let mut start = 0;
        loop {
            let indent = if start == 0 { 0 } else { hang };
            let end = row_end(text, start, columns - indent, &breaks);
            rows.push(Row {
                line: index,
                range: start..end,
                indent,
            });
            if end == text.len() {
                break;
            }
            start = end;
        }
    }
    rows
}

/// Where a row of `text` may end, best first: after a space or a comma outside a string, past
/// the line's `lead` characters of indentation; then where a string, number or name starts.
fn breaks(text: &str, lead: usize) -> [Vec<usize>; 2] {
    let lead = text
        .char_indices()
        .nth(lead)
        .map_or(text.len(), |(at, _)| at);
    let mut separators = Vec::new();
    let mut parts = Vec::new();
    let mut offset = 0;
    for (token, part) in tokens(text) {
        if token == Token::Plain {
            separators.extend(
                part.char_indices()
                    .filter(|&(_, c)| c == ' ' || c == ',')
                    .map(|(at, c)| offset + at + c.len_utf8())
                    .filter(|&at| at > lead),
            );
        } else {
            parts.push(offset);
        }
        offset += part.len();
    }
    [separators, parts]
}

/// The end of the row of `text` that starts at `start` and holds `width` characters.
fn row_end(text: &str, start: usize, width: usize, breaks: &[Vec<usize>; 2]) -> usize {
    let Some((full, _)) = text[start..].char_indices().nth(width) else {
        return text.len();
    };
    let full = start + full;
    breaks
        .iter()
        .find_map(|ends| {
            ends.iter()
                .rev()
                .copied()
                .find(|&end| start < end && end <= full)
        })
        .unwrap_or(full)
}

/// The first row a box of `rows` rows shows when asked to start at `top`: never past the
/// point where the last row reaches its bottom.
pub fn first_row(top: isize, rows: usize, total: usize) -> usize {
    top.clamp(0, total.saturating_sub(rows) as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_is_coloured_as_its_name_its_value_and_its_punctuation() {
        assert_eq!(
            tokens(r#"  "nominalDailyTarget": "07:24:00","#),
            [
                (Token::Plain, "  "),
                (Token::Key, r#""nominalDailyTarget""#),
                (Token::Plain, ": "),
                (Token::String, r#""07:24:00""#),
                (Token::Plain, ","),
            ]
        );
        assert_eq!(
            tokens(r#"    "maximumWeeklyDebt": null"#),
            [
                (Token::Plain, "    "),
                (Token::Key, r#""maximumWeeklyDebt""#),
                (Token::Plain, ": "),
                (Token::Literal, "null"),
            ]
        );
        assert_eq!(
            tokens(r#""schemaVersion": -1.5e3, "a\"b" }"#),
            [
                (Token::Key, r#""schemaVersion""#),
                (Token::Plain, ": "),
                (Token::Literal, "-1.5e3"),
                (Token::Plain, ", "),
                (Token::String, r#""a\"b""#),
                (Token::Plain, " }"),
            ]
        );
        assert_eq!(tokens("},"), [(Token::Plain, "},")]);
    }

    #[test]
    fn the_lines_drop_the_byte_order_mark_and_the_line_breaks() {
        assert_eq!(
            lines("\u{feff}{\r\n  \"usagePing\": true\r\n}\r\n"),
            ["{", "  \"usagePing\": true", "}"]
        );
        assert_eq!(lines("{\n}"), ["{", "}"]);
        assert!(lines("").is_empty());
    }

    #[test]
    fn the_box_scrolls_between_the_first_row_and_the_last_row_at_its_bottom() {
        assert_eq!(first_row(-3, 29, 52), 0);
        assert_eq!(first_row(10, 29, 52), 10);
        assert_eq!(first_row(40, 29, 52), 23);
        assert_eq!(first_row(5, 29, 12), 0);
    }

    /// The rows as the box shows them: the line's number on its first row, then the indent and
    /// the text.
    fn shown(lines: &[&str], columns: usize) -> Vec<String> {
        let lines: Vec<String> = lines.iter().map(|&line| line.to_owned()).collect();
        rows(&lines, columns)
            .into_iter()
            .map(|row| {
                let number = if row.starts_line() {
                    (row.line + 1).to_string()
                } else {
                    String::new()
                };
                let indent = " ".repeat(row.indent);
                format!("{number:>2}|{indent}{}", &lines[row.line][row.range])
            })
            .collect()
    }

    #[test]
    fn a_line_goes_on_after_a_space_with_a_hanging_indent() {
        assert_eq!(
            shown(
                &[
                    r#"  "notifications": { "departure": { "enabled": true, "leadTime": "00:15:00" } }"#
                ],
                40
            ),
            [
                r#" 1|  "notifications": { "departure": { "#,
                r#"  |    "enabled": true, "leadTime": "#,
                r#"  |    "00:15:00" } }"#,
            ]
        );
    }

    #[test]
    fn a_minified_file_goes_on_after_its_commas() {
        assert_eq!(
            shown(
                &[r#"{"schemaVersion":1,"nominalDailyTarget":"07:24:00","usagePing":true}"#],
                24
            ),
            [
                r#" 1|{"schemaVersion":1,"#,
                r#"  |  "nominalDailyTarget":"#,
                r#"  |  "07:24:00","#,
                r#"  |  "usagePing":true}"#,
            ]
        );
    }

    #[test]
    fn short_and_empty_lines_keep_one_row_and_a_line_of_the_width_exactly_one() {
        assert_eq!(
            shown(&["{", "", "  \"a\": 1", "}"], 8),
            [" 1|{", " 2|", " 3|  \"a\": 1", " 4|}"]
        );
        assert_eq!(shown(&["0123456789"], 10), [" 1|0123456789"]);
    }

    #[test]
    fn only_a_part_longer_than_a_row_is_cut_where_the_row_ends() {
        assert_eq!(
            shown(&[r#""note": "Détection intégrée""#], 12),
            [r#" 1|"note": "#, r#"  |  "Détection"#, r#"  |   intégrée""#,]
        );
        assert_eq!(shown(&["0123456789"], 6), [" 1|012345", "  |  6789"]);
    }

    #[test]
    fn a_long_string_keeps_its_colour_on_every_row() {
        let line = r#"  "folder": "C:\\Users\\Camille\\AppData\\Local\\WorkTime","#;
        let lines = [line.to_owned()];
        let parts: Vec<_> = rows(&lines, 24)
            .into_iter()
            .map(|row| tokens_within(line, row.range))
            .collect();
        assert_eq!(
            parts,
            [
                vec![
                    (Token::Plain, "  "),
                    (Token::Key, r#""folder""#),
                    (Token::Plain, ": "),
                ],
                vec![(Token::String, r#""C:\\Users\\Camille\"#)],
                vec![(Token::String, r#"\AppData\\Local\\Wor"#)],
                vec![(Token::String, r#"kTime""#), (Token::Plain, ",")],
            ]
        );
    }
}
