//! The `license` field of a Cargo manifest: an SPDX expression, or the older `MIT/Apache-2.0`
//! form, where a slash means OR.

/// The licenses whose terms a notice in the download satisfies.
pub const ALLOWED: &[&str] = &[
    "MIT",
    "MIT-0",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "0BSD",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "BSL-1.0",
    "CC0-1.0",
    "Unlicense",
    // webpki-root-certs: its one condition is the agreement text alongside the data.
    "CDLA-Permissive-2.0",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Expression {
    License(String),
    And(Vec<Expression>),
    Or(Vec<Expression>),
}

impl Expression {
    pub fn parse(text: &str) -> Result<Self, String> {
        let tokens = tokenize(text);
        let mut parser = Parser {
            tokens: &tokens,
            position: 0,
        };
        let expression = parser.or()?;
        match parser.tokens.get(parser.position) {
            None => Ok(expression),
            Some(token) => Err(format!("unexpected {token:?}")),
        }
    }

    pub fn is_allowed(&self) -> bool {
        self.is_satisfied_by(&|id| ALLOWED.contains(&id))
    }

    /// One satisfied branch is enough for OR, every term must be satisfied for AND.
    pub fn is_satisfied_by(&self, satisfies: &impl Fn(&str) -> bool) -> bool {
        match self {
            Self::License(id) => satisfies(id),
            Self::And(terms) => terms.iter().all(|term| term.is_satisfied_by(satisfies)),
            Self::Or(branches) => branches
                .iter()
                .any(|branch| branch.is_satisfied_by(satisfies)),
        }
    }

    /// Every license the expression names, in order.
    pub fn licenses(&self) -> Vec<&str> {
        match self {
            Self::License(id) => vec![id.as_str()],
            Self::And(terms) | Self::Or(terms) => terms.iter().flat_map(Self::licenses).collect(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    And,
    Or,
    Id(String),
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, tokens: &mut Vec<Token>| {
        match word.as_str() {
            "" => return,
            "AND" => tokens.push(Token::And),
            "OR" => tokens.push(Token::Or),
            id => tokens.push(Token::Id(id.to_owned())),
        }
        word.clear();
    };
    for character in text.chars() {
        let token = match character {
            '(' => Some(Token::Open),
            ')' => Some(Token::Close),
            '/' => Some(Token::Or),
            c if c.is_whitespace() => None,
            c => {
                word.push(c);
                continue;
            }
        };
        flush(&mut word, &mut tokens);
        tokens.extend(token);
    }
    flush(&mut word, &mut tokens);
    tokens
}

/// AND binds tighter than OR, as in SPDX.
struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
}

impl Parser<'_> {
    fn or(&mut self) -> Result<Expression, String> {
        let mut branches = vec![self.and()?];
        while self.eat(&Token::Or) {
            branches.push(self.and()?);
        }
        Ok(single_or(branches, Expression::Or))
    }

    fn and(&mut self) -> Result<Expression, String> {
        let mut terms = vec![self.atom()?];
        while self.eat(&Token::And) {
            terms.push(self.atom()?);
        }
        Ok(single_or(terms, Expression::And))
    }

    fn atom(&mut self) -> Result<Expression, String> {
        let token = self.tokens.get(self.position);
        self.position += 1;
        match token {
            Some(Token::Id(id)) => Ok(Expression::License(id.clone())),
            Some(Token::Open) => {
                let inner = self.or()?;
                if self.eat(&Token::Close) {
                    Ok(inner)
                } else {
                    Err("unclosed parenthesis".to_owned())
                }
            }
            Some(token) => Err(format!("unexpected {token:?}")),
            None => Err("unexpected end".to_owned()),
        }
    }

    fn eat(&mut self, expected: &Token) -> bool {
        let matches = self.tokens.get(self.position) == Some(expected);
        if matches {
            self.position += 1;
        }
        matches
    }
}

fn single_or(mut items: Vec<Expression>, wrap: fn(Vec<Expression>) -> Expression) -> Expression {
    if items.len() == 1 {
        items.remove(0)
    } else {
        wrap(items)
    }
}

#[cfg(test)]
mod tests {
    use super::Expression::{self, And, License, Or};

    fn id(name: &str) -> Expression {
        License(name.to_owned())
    }

    fn parse(text: &str) -> Expression {
        Expression::parse(text).unwrap()
    }

    #[test]
    fn parses_a_single_license() {
        assert_eq!(parse("MIT"), id("MIT"));
    }

    #[test]
    fn reads_a_slash_as_or() {
        assert_eq!(
            parse("MIT/Apache-2.0"),
            Or(vec![id("MIT"), id("Apache-2.0")])
        );
    }

    #[test]
    fn binds_and_tighter_than_or() {
        assert_eq!(
            parse("MIT OR Apache-2.0 AND BSD-3-Clause"),
            Or(vec![
                id("MIT"),
                And(vec![id("Apache-2.0"), id("BSD-3-Clause")])
            ])
        );
    }

    #[test]
    fn groups_with_parentheses() {
        assert_eq!(
            parse("(MIT OR Apache-2.0) AND Unicode-3.0"),
            And(vec![
                Or(vec![id("MIT"), id("Apache-2.0")]),
                id("Unicode-3.0")
            ])
        );
    }

    #[test]
    fn rejects_malformed_expressions() {
        for text in [
            "",
            "MIT OR",
            "(MIT",
            "MIT)",
            "MIT Apache-2.0",
            "MIT AND OR ISC",
        ] {
            assert!(Expression::parse(text).is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn or_needs_one_allowed_branch() {
        assert!(parse("GPL-3.0-only OR MIT").is_allowed());
        assert!(parse("Apache-2.0/MIT").is_allowed());
        assert!(!parse("GPL-3.0-only OR LGPL-2.1-or-later").is_allowed());
    }

    #[test]
    fn and_needs_every_term_allowed() {
        assert!(parse("Apache-2.0 AND MIT").is_allowed());
        assert!(parse("(MIT OR Apache-2.0) AND Unicode-3.0").is_allowed());
        assert!(!parse("MIT AND MPL-2.0").is_allowed());
    }

    #[test]
    fn rejects_a_license_outside_the_list() {
        assert!(!parse("MPL-2.0").is_allowed());
        assert!(!parse("mit").is_allowed());
    }

    #[test]
    fn lists_every_named_license() {
        assert_eq!(
            parse("(MIT OR Apache-2.0) AND Zlib").licenses(),
            ["MIT", "Apache-2.0", "Zlib"]
        );
    }
}
