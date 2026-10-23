//! Writes THIRD-PARTY-NOTICES.txt for the release worktime.exe: every crate compiled into it from
//! outside the workspace (a registry or a git source), with its license and the license texts it
//! ships.
//!
//! The crate set is `cargo tree` for the Windows target with worktime-app's `release` feature,
//! over normal edges only: build scripts and proc-macros run on the build machine and are not in
//! the exe.
//! `cargo metadata` resolves features for the whole workspace and would list crates the exe never
//! links, so it only supplies each crate's manifest path, license and repository. It runs with
//! every feature on, so that it knows every crate the tree names.
//!
//! Every license file next to a crate's manifest is included, whichever branch of its license the
//! notice relies on. A crate that ships none gets the standard texts from packaging/licenses,
//! which must cover its whole expression, and its authors as the copyright holders the MIT and
//! BSD texts require.
//!
//! Usage: `cargo run --locked -p worktime-notices -- <output file>`. Fails, naming the crate, on a
//! missing license, a license outside [`license::ALLOWED`] or license texts that do not cover it.

mod license;

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use license::Expression;
use serde_json::Value;

const TARGET: &str = "x86_64-pc-windows-msvc";
const LICENSE_FILE_PREFIXES: &[&str] = &["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"];
const SQLITE_NOTICE: &str = "\
libsqlite3-sys compiles the SQLite library it bundles into worktime.exe. SQLite is
in the public domain (https://sqlite.org/copyright.html). Its source carries this
blessing in place of a license:

    The author disclaims copyright to this source code. In place of
    a legal notice, here is a blessing:

       May you do good and not evil.
       May you find forgiveness for yourself and forgive others.
       May you share freely, never taking more than you give.
";

struct Crate {
    name: String,
    version: String,
    license: String,
    repository: Option<String>,
    /// Only for a crate covered by the standard texts, which name no copyright holder.
    copyright: Option<String>,
    texts: Vec<String>,
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(output), None) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: worktime-notices <output file>");
        return ExitCode::FAILURE;
    };
    match generate(Path::new(&output)) {
        Ok(count) => {
            eprintln!("{}: {count} crates", Path::new(&output).display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("worktime-notices: {error}");
            ExitCode::FAILURE
        }
    }
}

fn generate(output: &Path) -> Result<usize, String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = workspace.join("Cargo.toml");
    let tree = cargo(&[
        "tree",
        "--locked",
        "--manifest-path",
        path_str(&manifest)?,
        "--package",
        "worktime-app",
        "--features",
        "release",
        "--target",
        TARGET,
        "--edges",
        "normal,no-proc-macro",
        "--prefix",
        "none",
        "--format",
        "{p}",
    ])?;
    let metadata = cargo(&[
        "metadata",
        "--locked",
        "--all-features",
        "--format-version",
        "1",
        "--filter-platform",
        TARGET,
        "--manifest-path",
        path_str(&manifest)?,
    ])?;
    let metadata: Value =
        serde_json::from_str(&metadata).map_err(|error| format!("cargo metadata: {error}"))?;
    let packages: HashMap<(&str, &str), &Value> = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?
        .iter()
        .filter_map(|package| {
            Some((
                (package["name"].as_str()?, package["version"].as_str()?),
                package,
            ))
        })
        .collect();

    let mut crates = Vec::new();
    let mut rejected = Vec::new();
    for (name, version) in tree_packages(&tree)? {
        let package = packages
            .get(&(name, version))
            .ok_or_else(|| format!("{name} {version} is not in cargo metadata"))?;
        if package["source"].is_null() {
            continue;
        }
        let Some(license) = package["license"]
            .as_str()
            .filter(|license| !license.trim().is_empty())
        else {
            return Err(format!("{name} {version} declares no license"));
        };
        let expression = Expression::parse(license)
            .ok()
            .filter(Expression::is_allowed);
        let Some(expression) = expression else {
            rejected.push(format!("{name} {version} ({license:?})"));
            continue;
        };
        let directory = Path::new(package["manifest_path"].as_str().unwrap_or_default())
            .parent()
            .ok_or_else(|| format!("{name} {version}: no manifest path"))?;
        let mut texts = license_files(directory)?;
        let mut copyright = None;
        if texts.is_empty() {
            texts = standard_texts(&workspace.join("packaging/licenses"), &expression)?
                .ok_or_else(|| {
                    format!(
                        "{name} {version} ships no license file and packaging/licenses does not cover {license}"
                    )
                })?;
            copyright = Some(authors(package).unwrap_or_else(|| format!("the {name} authors")));
        }
        crates.push(Crate {
            name: name.to_owned(),
            version: version.to_owned(),
            license: license.to_owned(),
            repository: package["repository"].as_str().map(str::to_owned),
            copyright,
            texts,
        });
    }
    if !rejected.is_empty() {
        return Err(format!("licenses not allowed: {}", rejected.join(", ")));
    }

    let count = crates.len();
    fs::write(output, render(crates)).map_err(|error| format!("{}: {error}", output.display()))?;
    Ok(count)
}

fn cargo(arguments: &[&str]) -> Result<String, String> {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("cargo {}: {error}", arguments[0]))?;
    if !output.status.success() {
        return Err(format!(
            "cargo {} failed: {}",
            arguments[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("cargo {}: {error}", arguments[0]))
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("{} is not UTF-8", path.display()))
}

/// The `name vX.Y.Z` of each line of `cargo tree --prefix none --format {p}`, once each.
fn tree_packages(tree: &str) -> Result<BTreeSet<(&str, &str)>, String> {
    tree.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut words = line.split_whitespace();
            match (words.next(), words.next().and_then(|v| v.strip_prefix('v'))) {
                (Some(name), Some(version)) => Ok((name, version)),
                _ => Err(format!("cargo tree: unexpected line {line:?}")),
            }
        })
        .collect()
}

fn license_files(directory: &Path) -> Result<Vec<String>, String> {
    let entries =
        fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        let name = name.to_ascii_uppercase();
                        LICENSE_FILE_PREFIXES
                            .iter()
                            .any(|prefix| name.starts_with(prefix))
                    })
        })
        .collect();
    paths.sort();
    paths.iter().map(|path| read_text(path)).collect()
}

/// None when the texts in `directory` leave a term of `expression` unmet, such as the Unicode-3.0
/// of `(MIT OR Apache-2.0) AND Unicode-3.0`.
fn standard_texts(
    directory: &Path,
    expression: &Expression,
) -> Result<Option<Vec<String>>, String> {
    let path = |id: &str| directory.join(format!("{id}.txt"));
    if !expression.is_satisfied_by(&|id| path(id).is_file()) {
        return Ok(None);
    }
    expression
        .licenses()
        .into_iter()
        .map(path)
        .filter(|path| path.is_file())
        .map(|path| read_text(&path))
        .collect::<Result<_, _>>()
        .map(Some)
}

fn authors(package: &Value) -> Option<String> {
    let authors: Vec<&str> = package["authors"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    (!authors.is_empty()).then(|| authors.join(", "))
}

fn read_text(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    Ok(format!("{}\n", text.trim_end()))
}

fn render(mut crates: Vec<Crate>) -> String {
    crates.sort_by(|a, b| {
        (&a.name, version_key(&a.version)).cmp(&(&b.name, version_key(&b.version)))
    });

    let mut out = String::from(
        "THIRD-PARTY NOTICES FOR WORKTIME\n\n\
         worktime.exe contains the following third-party Rust crates. Each line gives the\n\
         crate, its version, its license as the crate declares it, and its repository. The\n\
         license texts follow, each one once, with the crates it covers. A crate that ships no\n\
         license file is covered by the standard text of its license, and a copyright line\n\
         under it names its authors.\n\n\
         Generated by crates/worktime-notices from Cargo.lock.\n\n",
    );
    for package in &crates {
        let repository = package
            .repository
            .as_deref()
            .unwrap_or("no repository listed");
        out += &format!(
            "{} {} — {} — {repository}\n",
            package.name, package.version, package.license
        );
        if let Some(copyright) = &package.copyright {
            out += &format!("    Copyright: {copyright}\n");
        }
    }

    if crates
        .iter()
        .any(|package| package.name == "libsqlite3-sys")
    {
        heading(&mut out, "SQLite");
        out += SQLITE_NOTICE;
    }

    // Copies of one license that differ only in whitespace are printed once, as first seen.
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for package in &crates {
        for text in &package.texts {
            let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let position = *index.entry(words).or_insert_with(|| {
                groups.push((text, Vec::new()));
                groups.len() - 1
            });
            let label = format!("{} {}", package.name, package.version);
            if !groups[position].1.contains(&label) {
                groups[position].1.push(label);
            }
        }
    }
    for (text, labels) in groups {
        heading(&mut out, &format!("Used by {}", labels.join(", ")));
        out += text;
    }
    out
}

const RULE: &str =
    "================================================================================";

/// The title between two rules, wrapped at the rule's width after a comma, so that no crate name
/// is split from its version.
fn heading(out: &mut String, title: &str) {
    *out += "\n\n";
    *out += RULE;
    *out += "\n";
    let mut line = String::new();
    for word in title.split_inclusive(", ").map(str::trim_end) {
        if !line.is_empty() && line.len() + 1 + word.len() > RULE.len() {
            *out += &line;
            *out += "\n";
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line += word;
    }
    *out += &line;
    *out += "\n";
    *out += RULE;
    *out += "\n\n";
}

/// Orders 0.9.0 before 0.10.0; anything after the numbers breaks ties as text.
fn version_key(version: &str) -> (Vec<u64>, &str) {
    let end = version.find(['-', '+']).unwrap_or(version.len());
    let numbers = version[..end]
        .split('.')
        .map(|part| part.parse().unwrap_or(u64::MAX))
        .collect();
    (numbers, &version[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str, texts: &[&str]) -> Crate {
        Crate {
            name: name.to_owned(),
            version: version.to_owned(),
            license: "MIT".to_owned(),
            repository: None,
            copyright: None,
            texts: texts.iter().map(|text| text.to_string()).collect(),
        }
    }

    #[test]
    fn lists_crates_by_name_then_version() {
        let out = render(vec![
            package("b", "1.0.0", &["B\n"]),
            package("a", "0.10.0", &["A\n"]),
            package("a", "0.9.1", &["A\n"]),
        ]);
        let lines: Vec<&str> = out.lines().filter(|line| line.contains(" — ")).collect();
        assert_eq!(
            lines,
            [
                "a 0.9.1 — MIT — no repository listed",
                "a 0.10.0 — MIT — no repository listed",
                "b 1.0.0 — MIT — no repository listed",
            ]
        );
    }

    #[test]
    fn prints_a_shared_text_once_for_every_crate_it_covers() {
        let out = render(vec![
            package("one", "1.0.0", &["SHARED TEXT\n", "OWN\n"]),
            package("two", "2.0.0", &["SHARED\n   TEXT\n"]),
        ]);
        assert_eq!(out.matches("SHARED").count(), 1);
        assert!(out.contains(&format!(
            "Used by one 1.0.0, two 2.0.0\n{RULE}\n\nSHARED TEXT\n"
        )));
        assert!(out.contains(&format!("Used by one 1.0.0\n{RULE}\n\nOWN\n")));
    }

    #[test]
    fn wraps_long_headings_at_the_rule() {
        let mut out = String::new();
        heading(
            &mut out,
            &format!("Used by {}", ["crate 1.0.0"; 20].join(", ")),
        );
        assert!(out.lines().all(|line| line.len() <= RULE.len()));
        assert_eq!(out.matches("crate 1.0.0").count(), 20);
    }

    #[test]
    fn adds_the_sqlite_notice_only_with_libsqlite3_sys() {
        assert!(!render(vec![package("rusqlite", "0.40.2", &["MIT\n"])]).contains("SQLite"));
        assert!(
            render(vec![package("libsqlite3-sys", "0.38.0", &["MIT\n"])]).contains("public domain")
        );
    }

    #[test]
    fn prints_the_copyright_of_a_crate_on_the_standard_text() {
        let mut covered = package("b", "1.0.0", &["MIT\n"]);
        covered.copyright = Some("Jane Doe <jane@example.com>".to_owned());
        let out = render(vec![package("a", "1.0.0", &["MIT\n"]), covered]);
        assert!(out.contains(
            "a 1.0.0 — MIT — no repository listed\n\
             b 1.0.0 — MIT — no repository listed\n    \
             Copyright: Jane Doe <jane@example.com>\n"
        ));
    }

    #[test]
    fn standard_texts_must_cover_every_and_term() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/licenses");
        let texts = |text: &str| {
            standard_texts(&directory, &Expression::parse(text).unwrap())
                .unwrap()
                .map(|texts| texts.len())
        };
        assert_eq!(texts("MIT OR Apache-2.0"), Some(2));
        assert_eq!(texts("MIT OR Zlib"), Some(1));
        assert_eq!(texts("(MIT OR Apache-2.0) AND Unicode-3.0"), None);
        assert_eq!(texts("Zlib"), None);
    }

    #[test]
    fn reads_each_tree_line_once() {
        let tree = "a v1.0.0\nb v0.2.0 (proc-macro)\na v1.0.0 (*)\nw v0.1.0 (/src/w)\n";
        assert_eq!(
            tree_packages(tree).unwrap().into_iter().collect::<Vec<_>>(),
            [("a", "1.0.0"), ("b", "0.2.0"), ("w", "0.1.0")]
        );
    }
}
