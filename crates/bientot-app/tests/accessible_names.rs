//! A screen reader announces a heading, a version, a group the Tab key reaches: none of them
//! may come out with an empty name. It expands and collapses a control that says it is
//! expanded, which only `widgets::expandable` makes work.
//!
//! GPUI names a node from its `aria_label`, never from its child text, and AccessKit names a
//! `Role::Label` from its value, not its label. The headless GPUI of the tests never activates
//! accessibility (`Window::is_a11y_active` stays false and `debug_a11y_tree_json` returns
//! `None`), so the tree cannot be walked here. This reads the builders instead.

use std::fs;
use std::path::{Path, PathBuf};

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The lines of the builder chain that `.role(..)` on `at` belongs to: the lines at its
/// indentation around it, and the line that opens it.
fn chain(lines: &[&str], at: usize) -> String {
    let level = indent(lines[at]);
    let mut chain = vec![lines[at]];
    for line in lines[..at].iter().rev() {
        if line.trim().is_empty() || indent(line) < level {
            chain.push(line);
            break;
        }
        if indent(line) == level {
            chain.push(line);
        }
    }
    for line in &lines[at + 1..] {
        if line.trim().is_empty() || indent(line) < level {
            break;
        }
        if indent(line) == level {
            chain.push(line);
        }
    }
    chain.join("\n")
}

#[test]
fn every_node_a_screen_reader_reads_or_drives_is_built_to_be() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let mut unnamed = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let lines: Vec<_> = text.lines().collect();
        for (at, line) in lines.iter().enumerate() {
            let place = || format!("{}:{}", file.strip_prefix(&root).unwrap().display(), at + 1);
            if line.contains(".role(Role::Heading)") && !file.ends_with("widgets.rs") {
                unnamed.push(format!(
                    "{}: a heading not built with `widgets::heading`, which names it",
                    place()
                ));
            }
            if line.contains(".aria_expanded(") && !file.ends_with("widgets.rs") {
                unnamed.push(format!(
                    "{}: `.aria_expanded(..)` alone, a screen reader cannot expand it: use `widgets::expandable`",
                    place()
                ));
            }
            if line.contains(".role(Role::Label)") && !chain(&lines, at).contains(".aria_value(") {
                unnamed.push(format!(
                    "{}: a label without a name, AccessKit names it from `.aria_value(..)`",
                    place()
                ));
            }
            if line.contains(".role(Role::Group)") && !chain(&lines, at).contains(".aria_label(") {
                unnamed.push(format!(
                    "{}: a group without a name, give it `.aria_label(..)`",
                    place()
                ));
            }
        }
    }
    assert!(unnamed.is_empty(), "\n{}", unnamed.join("\n"));
}
