//! JSON as `System.Text.Json` writes and reads it.
//!
//! [`JsonObject`] writes compact objects: properties in the order written, no whitespace.
//! Names from fixed lists and timestamps need no escaping; free text is escaped as the default
//! `JavaScriptEncoder` of `System.Text.Json` escapes it.
//!
//! [`JsonValue`] reads a log record with the leniency of `JsonDocument.Parse`, which checks the
//! grammar and the nesting depth but not what strings and numbers hold: a lone surrogate escape,
//! invalid UTF-8 inside a string or a number no type can hold still make a valid document. Only
//! the values the export reads are decoded, when it reads them.

use std::fmt::{Display, Write};

pub(crate) struct JsonObject {
    out: String,
    empty: bool,
}

impl JsonObject {
    pub(crate) fn write(build: impl FnOnce(&mut Self)) -> String {
        let mut object = Self {
            out: String::from("{"),
            empty: true,
        };
        build(&mut object);
        object.out.push('}');
        object.out
    }

    /// A number or a boolean.
    pub(crate) fn value(&mut self, property: &str, value: impl Display) {
        self.property(property);
        write!(self.out, "{value}").expect("writing to a String cannot fail");
    }

    /// A string that needs no escaping: an enum name, an event name or a timestamp.
    pub(crate) fn name(&mut self, property: &str, name: &str) {
        debug_assert!(!name.contains(['"', '\\']) && name.is_ascii());
        self.property(property);
        write!(self.out, "\"{name}\"").expect("writing to a String cannot fail");
    }

    /// Free text, escaped as `System.Text.Json` escapes it by default: `"` and the HTML-sensitive
    /// characters `&'+<>` and `` ` `` as `\uXXXX`, the usual short escapes for `\\` and the
    /// controls that have one, and every other control and non-ASCII character as one `\uXXXX`
    /// per UTF-16 unit, in upper-case hexadecimal.
    pub(crate) fn string(&mut self, property: &str, text: &str) {
        self.property(property);
        self.out.push('"');
        for character in text.chars() {
            match character {
                '\\' => self.out.push_str("\\\\"),
                '\n' => self.out.push_str("\\n"),
                '\r' => self.out.push_str("\\r"),
                '\t' => self.out.push_str("\\t"),
                '\u{8}' => self.out.push_str("\\b"),
                '\u{c}' => self.out.push_str("\\f"),
                ' '..='~' if !matches!(character, '"' | '&' | '\'' | '+' | '<' | '>' | '`') => {
                    self.out.push(character);
                }
                _ => {
                    for unit in character.encode_utf16(&mut [0; 2]) {
                        write!(self.out, "\\u{unit:04X}").expect("writing to a String cannot fail");
                    }
                }
            }
        }
        self.out.push('"');
    }

    pub(crate) fn names<'a>(&mut self, property: &str, names: impl IntoIterator<Item = &'a str>) {
        self.property(property);
        self.out.push('[');
        for (index, name) in names.into_iter().enumerate() {
            if index > 0 {
                self.out.push(',');
            }
            write!(self.out, "\"{name}\"").expect("writing to a String cannot fail");
        }
        self.out.push(']');
    }

    pub(crate) fn object(&mut self, property: &str, build: impl FnOnce(&mut Self)) {
        self.property(property);
        let nested = Self::write(build);
        self.out.push_str(&nested);
    }

    fn property(&mut self, property: &str) {
        if !self.empty {
            self.out.push(',');
        }
        self.empty = false;
        write!(self.out, "\"{property}\":").expect("writing to a String cannot fail");
    }
}

pub(crate) enum JsonValue<'a> {
    /// The members in document order; a name may repeat.
    Object(Vec<(JsonString<'a>, JsonValue<'a>)>),
    Array,
    String(JsonString<'a>),
    /// The number's text, which follows the JSON grammar.
    Number(&'a [u8]),
    Literal,
}

/// A string's bytes between its quotes, escapes still in.
#[derive(Clone, Copy)]
pub(crate) struct JsonString<'a>(&'a [u8]);

impl<'a> JsonValue<'a> {
    /// The document in `bytes`, or `None` when it breaks the grammar or nests objects and arrays
    /// more than `max_depth` deep.
    pub(crate) fn parse(bytes: &'a [u8], max_depth: usize) -> Option<Self> {
        let mut parser = Parser {
            bytes,
            at: 0,
            max_depth,
        };
        parser.skip_whitespace();
        let value = parser.value(0)?;
        parser.skip_whitespace();
        (parser.at == bytes.len()).then_some(value)
    }

    /// The value of the last member named `name`, as `JsonElement.TryGetProperty` finds it.
    pub(crate) fn property(&self, name: &str) -> Option<&Self> {
        let Self::Object(members) = self else {
            return None;
        };
        members
            .iter()
            .rev()
            .find(|(member, _)| member.decode().as_deref() == Some(name))
            .map(|(_, value)| value)
    }

    pub(crate) fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// The string's text; `None` for another kind of value or a string that holds no valid
    /// Unicode text.
    pub(crate) fn string(&self) -> Option<String> {
        match self {
            Self::String(text) => text.decode(),
            _ => None,
        }
    }

    /// The number as `JsonElement.TryGetInt32` reads it: an integer within `i32`.
    pub(crate) fn int32(&self) -> Option<i32> {
        match self {
            Self::Number(text) => std::str::from_utf8(text).ok()?.parse().ok(),
            _ => None,
        }
    }
}

impl JsonString<'_> {
    fn decode(self) -> Option<String> {
        let mut bytes = Vec::with_capacity(self.0.len());
        let mut rest = self.0;
        while let Some((&byte, after)) = rest.split_first() {
            rest = after;
            if byte != b'\\' {
                bytes.push(byte);
                continue;
            }
            let (&escape, after) = rest.split_first()?;
            rest = after;
            let character = match escape {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\u{8}',
                b'f' => '\u{c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                _ => {
                    let high = hex4(rest)?;
                    rest = &rest[4..];
                    let code = if (0xD800..0xDC00).contains(&high) {
                        let low = rest.strip_prefix(b"\\u").and_then(hex4)?;
                        if !(0xDC00..0xE000).contains(&low) {
                            return None;
                        }
                        rest = &rest[6..];
                        0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                    } else {
                        high
                    };
                    char::from_u32(code)?
                }
            };
            bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
        }
        String::from_utf8(bytes).ok()
    }
}

/// Four hexadecimal digits at the start of `bytes`.
fn hex4(bytes: &[u8]) -> Option<u32> {
    let digits = bytes.get(..4)?;
    digits.iter().try_fold(0, |value, &digit| {
        Some(value * 16 + char::from(digit).to_digit(16)?)
    })
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    max_depth: usize,
}

impl<'a> Parser<'a> {
    /// A value inside `depth` objects and arrays.
    fn value(&mut self, depth: usize) -> Option<JsonValue<'a>> {
        match *self.bytes.get(self.at)? {
            b'{' if depth < self.max_depth => {
                self.at += 1;
                self.object(depth + 1)
            }
            b'[' if depth < self.max_depth => {
                self.at += 1;
                self.array(depth + 1)
            }
            b'"' => {
                self.at += 1;
                self.string().map(JsonValue::String)
            }
            b'-' | b'0'..=b'9' => self.number().map(JsonValue::Number),
            _ => {
                let literal = ["true", "false", "null"]
                    .into_iter()
                    .find(|literal| self.bytes[self.at..].starts_with(literal.as_bytes()))?;
                self.at += literal.len();
                Some(JsonValue::Literal)
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<JsonValue<'a>> {
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.eat(b'}') {
            return Some(JsonValue::Object(members));
        }
        loop {
            self.skip_whitespace();
            if !self.eat(b'"') {
                return None;
            }
            let name = self.string()?;
            self.skip_whitespace();
            if !self.eat(b':') {
                return None;
            }
            self.skip_whitespace();
            members.push((name, self.value(depth)?));
            self.skip_whitespace();
            if self.eat(b'}') {
                return Some(JsonValue::Object(members));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<JsonValue<'a>> {
        self.skip_whitespace();
        if self.eat(b']') {
            return Some(JsonValue::Array);
        }
        loop {
            self.skip_whitespace();
            self.value(depth)?;
            self.skip_whitespace();
            if self.eat(b']') {
                return Some(JsonValue::Array);
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    /// The rest of a string whose opening quote was read.
    fn string(&mut self) -> Option<JsonString<'a>> {
        let start = self.at;
        loop {
            match *self.bytes.get(self.at)? {
                b'"' => {
                    self.at += 1;
                    return Some(JsonString(&self.bytes[start..self.at - 1]));
                }
                b'\\' => {
                    self.at += 1;
                    match *self.bytes.get(self.at)? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.at += 1,
                        b'u' => {
                            hex4(self.bytes.get(self.at + 1..)?)?;
                            self.at += 5;
                        }
                        _ => return None,
                    }
                }
                byte if byte < 0x20 => return None,
                _ => self.at += 1,
            }
        }
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`
    fn number(&mut self) -> Option<&'a [u8]> {
        let start = self.at;
        self.eat(b'-');
        if !self.eat(b'0') && self.digits() == 0 {
            return None;
        }
        if self.eat(b'.') && self.digits() == 0 {
            return None;
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if self.digits() == 0 {
                return None;
            }
        }
        Some(&self.bytes[start..self.at])
    }

    fn digits(&mut self) -> usize {
        let count = self.bytes[self.at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        self.at += count;
        count
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.bytes.get(self.at) == Some(&byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }
}
