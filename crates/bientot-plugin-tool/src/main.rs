//! `bientot-plugin`: packs a plugin into a `.bientot-plugin` file, signs it, and checks it in
//! Bientôt's own runtime against the demo days. See `crates/bientot-plugin-sdk/README.md`.

use std::path::PathBuf;
use std::process::ExitCode;

mod check;
mod install;
mod keys;
mod pack;

const USAGE: &str = "\
usage:
  bientot-plugin pack <plugin folder> <module.wasm> [--out <file.bientot-plugin>]
  bientot-plugin keygen <key file>
  bientot-plugin sign <file.bientot-plugin> --key <key file> [--announce <public key>] [--handover <old key file>]
  bientot-plugin check <file.bientot-plugin> [--language en|fr] [--expect <folder>] [--log]
  bientot-plugin install <file.bientot-plugin> --data-dir <Bientôt data folder>
  bientot-plugin sign-revocations <revocations.json> --key <key file>

pack     checks the manifest, the locale files and the module's imports, and writes the package.
keygen   creates a signing key and prints its public key and fingerprint. Keep the key file safe:
         an update of a signed plugin must be signed by the same key, or by one it announced.
sign     signs the package with the key; --announce adds a key that may sign later versions,
         --handover lets a new key take over from the old key that signed the installed version.
check    installs the package as Bientôt would, runs it on every demo day and prints its cards;
         with --expect, compares them with the <language>.txt files of the folder.
install  installs the package in the data folder, turned on with what it asks for, as after
         the consent dialog; an update keeps a plugin that is off as it is. For measurements
         and tests, with Bientôt closed.
sign-revocations  signs the revocation list in place with the maintainer's offline key, after
         checking its fields; the list's version must grow with each publication.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let parsed = match Args::parse(rest) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match command.as_str() {
        "pack" => pack::run(&parsed),
        "keygen" => keys::keygen(&parsed),
        "sign" => keys::sign(&parsed),
        "check" => check::run(&parsed),
        "install" => install::run(&parsed),
        "sign-revocations" => keys::sign_revocations(&parsed),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Positional arguments and `--name value` options; `--log` takes no value.
pub struct Args {
    pub positional: Vec<String>,
    pub options: Vec<(String, Option<String>)>,
}

const FLAGS: &[&str] = &["--log"];
const OPTIONS: &[&str] = &[
    "--out",
    "--key",
    "--announce",
    "--handover",
    "--language",
    "--expect",
    "--data-dir",
];

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let mut positional = Vec::new();
        let mut options = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if FLAGS.contains(&arg.as_str()) {
                options.push((arg.clone(), None));
            } else if OPTIONS.contains(&arg.as_str()) {
                let value = iter.next().ok_or_else(|| format!("{arg} needs a value"))?;
                options.push((arg.clone(), Some(value.clone())));
            } else if arg.starts_with("--") {
                return Err(format!("unknown option {arg}"));
            } else {
                positional.push(arg.clone());
            }
        }
        Ok(Args {
            positional,
            options,
        })
    }

    pub fn path(&self, index: usize, what: &str) -> Result<PathBuf, String> {
        self.positional
            .get(index)
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing the {what}"))
    }

    pub fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn flag(&self, name: &str) -> bool {
        self.options.iter().any(|(n, _)| n == name)
    }
}

/// Reads a file, naming it in the error.
pub fn read(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}
