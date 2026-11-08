//! Builders for cards (contract `view-tree.md`). A card has at most 4 lines and 2 buttons; texts
//! are keys of the plugin's locale files, so the host draws them in the app language:
//!
//! ```
//! use bientot_plugin_sdk::prelude::*;
//!
//! let card = card()
//!     .line(row![text("longest"), duration(5400)])
//!     .line(text("streak").arg("count", "3").muted())
//!     .action(button("reset", text("reset-label")));
//! # let _: bientot_plugin_sdk::api::Card = card.into();
//! ```

use std::collections::BTreeMap;

use bientot_plugin_api::view::{Card, CardType, Node, Role, Text, Tone};
use jiff::Timestamp;

/// A card being built. [`crate::ui::show`] takes it as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardBuilder {
    lines: Vec<Node>,
    actions: Vec<Node>,
}

/// An empty card.
pub fn card() -> CardBuilder {
    CardBuilder::default()
}

impl CardBuilder {
    /// Adds a line. Bientôt draws the first four.
    pub fn line(mut self, node: impl Into<Node>) -> CardBuilder {
        self.lines.push(node.into());
        self
    }

    /// Adds a line when `node` is `Some`.
    pub fn line_if(self, node: Option<impl Into<Node>>) -> CardBuilder {
        match node {
            Some(node) => self.line(node),
            None => self,
        }
    }

    /// Adds a button to the action row, made with [`button`]. At most two.
    pub fn action(mut self, button: impl Into<Node>) -> CardBuilder {
        self.actions.push(button.into());
        self
    }

    pub fn build(self) -> Card {
        Card {
            kind: CardType::Card,
            lines: self.lines,
            actions: self.actions,
        }
    }
}

impl From<CardBuilder> for Card {
    fn from(builder: CardBuilder) -> Card {
        builder.build()
    }
}

/// A text of the plugin's locale files. `{name}` in the text is replaced by the argument `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextNode(Text);

/// The text `key`, in the `body` role.
pub fn text(key: &str) -> TextNode {
    TextNode(Text {
        key: key.to_owned(),
        args: BTreeMap::new(),
        role: Role::Body,
    })
}

impl TextNode {
    /// Fills `{name}` with `value`. At most 8 arguments, 200 characters in all.
    pub fn arg(mut self, name: &str, value: impl ToString) -> TextNode {
        self.0.args.insert(name.to_owned(), value.to_string());
        self
    }

    pub fn role(mut self, role: Role) -> TextNode {
        self.0.role = role;
        self
    }

    pub fn title(self) -> TextNode {
        self.role(Role::Title)
    }

    pub fn muted(self) -> TextNode {
        self.role(Role::Muted)
    }

    /// A figure, drawn with tabular digits.
    pub fn value(self) -> TextNode {
        self.role(Role::Value)
    }

    pub fn mono(self) -> TextNode {
        self.role(Role::Mono)
    }
}

impl From<TextNode> for Text {
    fn from(text: TextNode) -> Text {
        text.0
    }
}

impl From<TextNode> for Node {
    fn from(text: TextNode) -> Node {
        Node::Text(text.0)
    }
}

/// Nodes side by side on one line. [`crate::row!`] takes nodes of different types.
pub fn row<N: Into<Node>>(children: impl IntoIterator<Item = N>) -> Node {
    Node::Row {
        children: children.into_iter().map(Into::into).collect(),
    }
}

/// Nodes one under the other. [`crate::stack!`] takes nodes of different types.
pub fn stack<N: Into<Node>>(children: impl IntoIterator<Item = N>) -> Node {
    Node::Stack {
        children: children.into_iter().map(Into::into).collect(),
    }
}

/// A duration of `seconds`, between 0 and 7 days, in the app's format.
pub fn duration(seconds: i64) -> Node {
    Node::Duration {
        value: seconds,
        since: None,
    }
}

/// `seconds` plus the time elapsed since `since`: the host keeps it growing with its clock,
/// without waking the plugin.
pub fn duration_since(seconds: i64, since: Timestamp) -> Node {
    Node::Duration {
        value: seconds,
        since: Some(since),
    }
}

/// The time left until `until`, drawn by the host.
pub fn countdown(until: Timestamp) -> Node {
    Node::Countdown { until }
}

/// An instant, in the app's clock format and the user's zone.
pub fn time_at(at: Timestamp) -> Node {
    Node::Time { at }
}

/// A badge of the design system.
pub fn badge(text: TextNode, tone: Tone) -> Node {
    Node::Badge {
        text: text.into(),
        tone,
    }
}

/// A progress bar; `label` is its accessible name.
pub fn progress(value: f64, max: f64, label: TextNode) -> Node {
    Node::Progress {
        value,
        max,
        label: label.into(),
    }
}

/// A button that runs the manifest's `command`, for [`CardBuilder::action`].
pub fn button(command: &str, label: TextNode) -> Button {
    Button {
        command: command.to_owned(),
        arg: None,
        label: label.into(),
    }
}

/// A button being built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    command: String,
    arg: Option<String>,
    label: Text,
}

impl Button {
    /// The `arg` that `command.invoked` carries, at most 200 characters.
    pub fn arg(mut self, arg: impl ToString) -> Button {
        self.arg = Some(arg.to_string());
        self
    }
}

impl From<Button> for Node {
    fn from(button: Button) -> Node {
        Node::Button {
            command: button.command,
            arg: button.arg,
            label: button.label,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_serializes_as_the_contract_example() {
        let at: Timestamp = "2026-11-08T13:42:00Z".parse().unwrap();
        let built: Card = card()
            .line(crate::row![text("longest"), duration(5400)])
            .line(crate::row![text("since-break"), duration_since(0, at)])
            .action(button("toggle-range", text("show-week")).arg("week"))
            .into();
        let json = serde_json::to_value(&built).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "card",
                "lines": [
                    { "type": "row", "children": [
                        { "type": "text", "key": "longest", "role": "body" },
                        { "type": "duration", "value": 5400 }
                    ]},
                    { "type": "row", "children": [
                        { "type": "text", "key": "since-break", "role": "body" },
                        { "type": "duration", "value": 0, "since": "2026-11-08T13:42:00Z" }
                    ]}
                ],
                "actions": [
                    { "type": "button", "command": "toggle-range", "arg": "week",
                      "label": { "key": "show-week", "role": "body" } }
                ]
            })
        );
    }
}
