//! The event XML that `EvtRender` returns, read as far as a power history boundary needs it.
//!
//! The .NET port parses it with `XDocument`; this reader accepts the same well-formed subset the
//! event log produces (elements, attributes in either quote, character data, entity and
//! character references, comments, processing instructions and CDATA) and refuses anything else,
//! a DTD included, so that a record it cannot read makes the whole history unavailable.

use std::num::NonZeroU64;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

use super::{PowerHistoryBoundary, PowerHistoryBoundaryKind};

/// The boundary a record describes, or `None` when it is malformed or describes no boundary.
/// Suspend and resume happen at `TimeCreated`; boot and shutdown at the `StartTime` and
/// `StopTime` of their event data, which the kernel can log much later.
pub(super) fn parse_boundary(xml: &str) -> Option<PowerHistoryBoundary> {
    let event = parse_document(xml)?;
    let system = event.single("System")?;
    let provider = system.single("Provider")?.attribute("Name")?;
    let event_id = digits(&system.single("EventID")?.value())?;
    let record_id = NonZeroU64::new(digits(&system.single("EventRecordID")?.value())?)?;
    let created_at = instant(system.single("TimeCreated")?.attribute("SystemTime")?)?;
    let kind = PowerHistoryBoundaryKind::from_event(provider, u32::try_from(event_id).ok()?)?;
    let occurred_at = match kind {
        PowerHistoryBoundaryKind::Suspend | PowerHistoryBoundaryKind::Resume => created_at,
        PowerHistoryBoundaryKind::Boot => event_data(&event, "StartTime")?,
        PowerHistoryBoundaryKind::Shutdown => event_data(&event, "StopTime")?,
    };
    Some(PowerHistoryBoundary {
        kind,
        record_id,
        occurred_at,
    })
}

fn event_data(event: &Element, name: &str) -> Option<Timestamp> {
    let mut named = event
        .single("EventData")?
        .elements()
        .filter(|data| data.name == "Data" && data.attribute("Name") == Some(name));
    let data = named.next()?;
    if named.next().is_some() {
        return None;
    }
    instant(&data.value())
}

/// A decimal number without sign or white space, as `NumberStyles.None` reads it.
fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// An ISO 8601 instant; one without an offset is UTC, as the .NET parser assumes.
fn instant(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    text.parse().ok().or_else(|| {
        let local: DateTime = text.parse().ok()?;
        Some(local.to_zoned(TimeZone::UTC).ok()?.timestamp())
    })
}

/// Deeper nesting is refused rather than recursed into.
const MAX_DEPTH: usize = 32;

struct Element<'a> {
    /// The local name, without its namespace prefix.
    name: &'a str,
    attributes: Vec<(&'a str, String)>,
    children: Vec<Node<'a>>,
}

enum Node<'a> {
    Element(Element<'a>),
    Text(String),
}

impl<'a> Element<'a> {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(attribute, _)| *attribute == name)
            .map(|(_, value)| value.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &Element<'a>> {
        self.children.iter().filter_map(|child| match child {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    /// The only child element of that local name.
    fn single(&self, name: &str) -> Option<&Element<'a>> {
        let mut named = self.elements().filter(|element| element.name == name);
        let element = named.next()?;
        named.next().is_none().then_some(element)
    }

    /// All the character data inside, as `XElement.Value` concatenates it.
    fn value(&self) -> String {
        let mut value = String::new();
        self.append_value(&mut value);
        value
    }

    fn append_value(&self, value: &mut String) {
        for child in &self.children {
            match child {
                Node::Element(element) => element.append_value(value),
                Node::Text(text) => value.push_str(text),
            }
        }
    }
}

fn parse_document(xml: &str) -> Option<Element<'_>> {
    let mut reader = Reader {
        rest: xml.strip_prefix('\u{feff}').unwrap_or(xml),
    };
    reader.skip_misc()?;
    let root = reader.element(0)?;
    reader.skip_misc()?;
    reader.rest.is_empty().then_some(root)
}

struct Reader<'a> {
    rest: &'a str,
}

impl<'a> Reader<'a> {
    fn eat(&mut self, prefix: &str) -> bool {
        match self.rest.strip_prefix(prefix) {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    /// Skips past the next `terminator`, failing when there is none.
    fn skip_past(&mut self, terminator: &str) -> Option<&'a str> {
        let end = self.rest.find(terminator)?;
        let skipped = &self.rest[..end];
        self.rest = &self.rest[end + terminator.len()..];
        Some(skipped)
    }

    fn skip_whitespace(&mut self) -> bool {
        let trimmed = self.rest.trim_start_matches([' ', '\t', '\r', '\n']);
        let skipped = trimmed.len() != self.rest.len();
        self.rest = trimmed;
        skipped
    }

    /// White space, comments and processing instructions around the root element.
    fn skip_misc(&mut self) -> Option<()> {
        loop {
            self.skip_whitespace();
            if self.eat("<!--") {
                self.skip_past("-->")?;
            } else if self.eat("<?") {
                self.skip_past("?>")?;
            } else {
                return Some(());
            }
        }
    }

    fn name(&mut self) -> Option<&'a str> {
        let end = self
            .rest
            .find(|c: char| c.is_whitespace() || "/>=<\"'&".contains(c))
            .unwrap_or(self.rest.len());
        let (name, rest) = self.rest.split_at(end);
        if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit() || "-.:".contains(c)) {
            return None;
        }
        self.rest = rest;
        Some(name)
    }

    fn element(&mut self, depth: usize) -> Option<Element<'a>> {
        if depth == MAX_DEPTH || !self.eat("<") {
            return None;
        }
        let name = self.name()?;
        let mut element = Element {
            name: name.rsplit(':').next()?,
            attributes: Vec::new(),
            children: Vec::new(),
        };
        loop {
            let separated = self.skip_whitespace();
            if self.eat("/>") {
                return Some(element);
            }
            if self.eat(">") {
                break;
            }
            if !separated {
                return None;
            }
            let attribute = self.name()?;
            self.skip_whitespace();
            if !self.eat("=") {
                return None;
            }
            self.skip_whitespace();
            let quote = if self.eat("\"") {
                "\""
            } else if self.eat("'") {
                "'"
            } else {
                return None;
            };
            let raw = self.skip_past(quote)?;
            if raw.contains('<') || element.attribute(attribute).is_some() {
                return None;
            }
            let value = unescape(raw)?;
            element.attributes.push((attribute, value));
        }
        loop {
            if self.eat("</") {
                let closing = self.name()?;
                self.skip_whitespace();
                return (closing == name && self.eat(">")).then_some(element);
            } else if self.eat("<!--") {
                self.skip_past("-->")?;
            } else if self.eat("<![CDATA[") {
                let text = self.skip_past("]]>")?;
                element.children.push(Node::Text(text.to_owned()));
            } else if self.eat("<?") {
                self.skip_past("?>")?;
            } else if self.rest.starts_with('<') {
                element
                    .children
                    .push(Node::Element(self.element(depth + 1)?));
            } else {
                let end = self.rest.find('<')?;
                let (text, rest) = self.rest.split_at(end);
                element.children.push(Node::Text(unescape(text)?));
                self.rest = rest;
            }
        }
    }
}

/// Replaces the predefined entities and character references; any other `&` is malformed.
fn unescape(text: &str) -> Option<String> {
    let mut unescaped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        unescaped.push_str(&rest[..start]);
        let end = rest[start..].find(';')? + start;
        let reference = &rest[start + 1..end];
        let character = match reference {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = match reference.strip_prefix("#x") {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => reference.strip_prefix('#')?.parse().ok()?,
                };
                char::from_u32(code)?
            }
        };
        unescaped.push(character);
        rest = &rest[end + 1..];
    }
    unescaped.push_str(rest);
    Some(unescaped)
}
