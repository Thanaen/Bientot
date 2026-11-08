//! Cards (contract `view-tree.md`). A plugin describes a card as a tree of primitives; the host
//! checks it, resolves its texts for the app language and draws it with its own components.

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::locale::TextLookup;
use crate::manifest::Manifest;

pub const MAX_DEPTH: usize = 6;
pub const MAX_NODES: usize = 64;
pub const MAX_LINES: usize = 4;
pub const MAX_ACTIONS: usize = 2;
pub const MAX_TEXT_CHARS: usize = 200;
pub const MAX_ARGS: usize = 8;
pub const MAX_DURATION_SECONDS: i64 = 7 * 24 * 3600;

/// The root of a card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    #[serde(rename = "type")]
    pub kind: CardType,
    #[serde(default)]
    pub lines: Vec<Node>,
    #[serde(default)]
    pub actions: Vec<Node>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardType {
    Card,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    Stack {
        children: Vec<Node>,
    },
    Row {
        children: Vec<Node>,
    },
    Text(Text),
    Duration {
        /// Seconds.
        value: i64,
        /// With `since`, the host adds the time elapsed since then at each render.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<Timestamp>,
    },
    Countdown {
        until: Timestamp,
    },
    Time {
        at: Timestamp,
    },
    Badge {
        text: Text,
        tone: Tone,
    },
    Progress {
        value: f64,
        max: f64,
        label: Text,
    },
    Button {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg: Option<String>,
        label: Text,
    },
    /// A primitive of a later api.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub key: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, String>,
    #[serde(default)]
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Title,
    #[default]
    Body,
    Muted,
    Value,
    Mono,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Info,
    Success,
    Warning,
    Error,
    Muted,
    #[serde(other)]
    Unknown,
}

/// A card that passed validation, its texts resolved for one language. The host draws this.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCard {
    pub lines: Vec<Prepared>,
    pub actions: Vec<PreparedButton>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    Stack(Vec<Prepared>),
    Row(Vec<Prepared>),
    Text {
        text: String,
        role: Role,
    },
    Duration {
        value: i64,
        since: Option<Timestamp>,
    },
    Countdown {
        until: Timestamp,
    },
    Time {
        at: Timestamp,
    },
    Badge {
        text: String,
        tone: Tone,
    },
    Progress {
        value: f64,
        max: f64,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedButton {
    pub command: String,
    pub arg: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewError {
    Malformed(String),
    TooDeep,
    TooManyNodes,
    TooManyActions,
    TooManyArgs,
    TextTooLong(String),
    ArgTooLong(String),
    UnknownText(String),
    UnknownCommand(String),
    UnknownPrimitive,
    ButtonOutsideActions,
    NotAButton,
    BadDuration,
    BadProgress,
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewError::Malformed(detail) => {
                write!(f, "the card is not a valid view tree: {detail}")
            }
            ViewError::TooDeep => write!(f, "the card is deeper than {MAX_DEPTH} levels"),
            ViewError::TooManyNodes => write!(f, "the card has more than {MAX_NODES} nodes"),
            ViewError::TooManyActions => write!(f, "the card has more than {MAX_ACTIONS} buttons"),
            ViewError::TooManyArgs => write!(f, "a text has more than {MAX_ARGS} arguments"),
            ViewError::TextTooLong(key) => write!(
                f,
                "the text `{key}` is longer than {MAX_TEXT_CHARS} characters"
            ),
            ViewError::ArgTooLong(command) => write!(
                f,
                "the `arg` of the button for `{command}` is longer than {MAX_TEXT_CHARS} characters"
            ),
            ViewError::UnknownText(key) => write!(f, "the text `{key}` is in no locale file"),
            ViewError::UnknownCommand(name) => {
                write!(f, "the command `{name}` is not in the manifest")
            }
            ViewError::UnknownPrimitive => write!(
                f,
                "the card uses a primitive this version of Bientôt does not have"
            ),
            ViewError::ButtonOutsideActions => write!(f, "buttons go in `actions`, not in `lines`"),
            ViewError::NotAButton => write!(f, "`actions` holds only buttons"),
            ViewError::BadDuration => write!(f, "a duration is negative or longer than 7 days"),
            ViewError::BadProgress => write!(f, "a progress bar needs 0 ≤ value ≤ max and max > 0"),
        }
    }
}

impl std::error::Error for ViewError {}

/// What validation needs from the plugin: its texts for the app language and its manifest.
pub struct ViewContext<'a> {
    pub texts: TextLookup<'a>,
    pub manifest: &'a Manifest,
}

impl Card {
    /// Reads a card from the JSON a plugin sent.
    pub fn from_value(value: serde_json::Value) -> Result<Card, ViewError> {
        serde_json::from_value(value).map_err(|e| ViewError::Malformed(e.to_string()))
    }

    /// Checks the whole tree and resolves its texts. Lines past the fourth are dropped; any other
    /// fault refuses the whole card, so that the host keeps the previous one.
    pub fn prepare(&self, context: &ViewContext) -> Result<PreparedCard, ViewError> {
        let mut walk = Walk { context, nodes: 1 };
        if self.actions.len() > MAX_ACTIONS {
            return Err(ViewError::TooManyActions);
        }
        let mut lines = Vec::with_capacity(self.lines.len().min(MAX_LINES));
        for node in &self.lines {
            let prepared = walk.node(node, 1)?;
            if lines.len() < MAX_LINES {
                lines.push(prepared);
            }
        }
        let mut actions = Vec::with_capacity(self.actions.len());
        for node in &self.actions {
            walk.count()?;
            let Node::Button {
                command,
                arg,
                label,
            } = node
            else {
                return Err(ViewError::NotAButton);
            };
            if context.manifest.command(command).is_none() {
                return Err(ViewError::UnknownCommand(command.clone()));
            }
            if arg
                .as_ref()
                .is_some_and(|a| a.chars().count() > MAX_TEXT_CHARS)
            {
                return Err(ViewError::ArgTooLong(command.clone()));
            }
            walk.count()?;
            actions.push(PreparedButton {
                command: command.clone(),
                arg: arg.clone(),
                label: walk.text(label)?,
            });
        }
        Ok(PreparedCard { lines, actions })
    }
}

struct Walk<'a, 'b> {
    context: &'a ViewContext<'b>,
    nodes: usize,
}

impl Walk<'_, '_> {
    fn count(&mut self) -> Result<(), ViewError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(ViewError::TooManyNodes);
        }
        Ok(())
    }

    fn node(&mut self, node: &Node, depth: usize) -> Result<Prepared, ViewError> {
        if depth > MAX_DEPTH {
            return Err(ViewError::TooDeep);
        }
        self.count()?;
        Ok(match node {
            Node::Stack { children } => Prepared::Stack(self.children(children, depth)?),
            Node::Row { children } => Prepared::Row(self.children(children, depth)?),
            Node::Text(text) => Prepared::Text {
                text: self.text(text)?,
                role: text.role,
            },
            Node::Duration { value, since } => {
                check_duration(*value)?;
                Prepared::Duration {
                    value: *value,
                    since: *since,
                }
            }
            Node::Countdown { until } => Prepared::Countdown { until: *until },
            Node::Time { at } => Prepared::Time { at: *at },
            Node::Badge { text, tone } => {
                self.count()?;
                Prepared::Badge {
                    text: self.text(text)?,
                    tone: *tone,
                }
            }
            Node::Progress { value, max, label } => {
                if !(value.is_finite()
                    && max.is_finite()
                    && *max > 0.0
                    && *value >= 0.0
                    && value <= max)
                {
                    return Err(ViewError::BadProgress);
                }
                self.count()?;
                Prepared::Progress {
                    value: *value,
                    max: *max,
                    label: self.text(label)?,
                }
            }
            Node::Button { .. } => return Err(ViewError::ButtonOutsideActions),
            Node::Unknown => return Err(ViewError::UnknownPrimitive),
        })
    }

    fn children(&mut self, children: &[Node], depth: usize) -> Result<Vec<Prepared>, ViewError> {
        children
            .iter()
            .map(|child| self.node(child, depth + 1))
            .collect()
    }

    fn text(&mut self, text: &Text) -> Result<String, ViewError> {
        if text.args.len() > MAX_ARGS {
            return Err(ViewError::TooManyArgs);
        }
        let template = self
            .context
            .texts
            .get(&text.key)
            .ok_or_else(|| ViewError::UnknownText(text.key.clone()))?;
        let resolved = substitute(template, &text.args);
        let args_chars: usize = text.args.values().map(|v| v.chars().count()).sum();
        if resolved.chars().count() > MAX_TEXT_CHARS || args_chars > MAX_TEXT_CHARS {
            return Err(ViewError::TextTooLong(text.key.clone()));
        }
        Ok(resolved)
    }
}

fn check_duration(seconds: i64) -> Result<(), ViewError> {
    if (0..=MAX_DURATION_SECONDS).contains(&seconds) {
        Ok(())
    } else {
        Err(ViewError::BadDuration)
    }
}

/// Replaces `{name}` by `args["name"]`. Nothing else is interpreted: an unknown `{name}` stays.
pub fn substitute(template: &str, args: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if args.contains_key(&after[..close]) => {
                out.push_str(&args[&after[..close]]);
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
