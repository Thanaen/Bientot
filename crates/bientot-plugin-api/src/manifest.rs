//! `plugin.json`, the manifest (contract `manifest.md`). It is read by hand from a JSON value
//! rather than derived, so that each refusal names its field and rule in the user's language.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::id::PluginId;
use crate::text::{Language, Localized};
use crate::{API_MAJOR, API_MINOR, Version};

pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const DEFAULT_MEMORY: u32 = 8 << 20;
pub const DEFAULT_MEMORY_MIN: u32 = 1 << 20;
pub const MAX_MEMORY: u32 = 32 << 20;
pub const MAX_COMMANDS: usize = 8;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_DESCRIPTION_CHARS: usize = 120;
pub const MAX_TITLE_CHARS: usize = 48;

/// What a plugin may reach. Each maps to one host interface (data model, « Capability »).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "read:day")]
    ReadDay,
    #[serde(rename = "ui:flyout")]
    UiFlyout,
}

impl Capability {
    pub const ALL: [Capability; 2] = [Capability::ReadDay, Capability::UiFlyout];

    pub fn name(self) -> &'static str {
        match self {
            Capability::ReadDay => "read:day",
            Capability::UiFlyout => "ui:flyout",
        }
    }

    /// The host interface the capability opens.
    pub fn interface(self) -> &'static str {
        match self {
            Capability::ReadDay => "bt:day@1",
            Capability::UiFlyout => "bt:ui@1",
        }
    }

    /// The consent text. It always comes from the host, never from a plugin.
    pub fn consent(self, language: Language) -> &'static str {
        match (self, language) {
            (Capability::ReadDay, Language::Fr) => "Lire votre journée en cours",
            (Capability::ReadDay, _) => "Read your current day",
            (Capability::UiFlyout, Language::Fr) => "Afficher une carte dans le flyout",
            (Capability::UiFlyout, _) => "Show a card in the flyout",
        }
    }

    fn parse(text: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.name() == text)
    }
}

/// The events a plugin can ask for. `installed`, `upgraded`, `activate` and `command.invoked` are
/// always delivered and are not listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EventKind {
    #[serde(rename = "day.refreshed")]
    DayRefreshed,
    #[serde(rename = "language.changed")]
    LanguageChanged,
}

impl EventKind {
    fn parse(text: &str) -> Option<EventKind> {
        match text {
            "day.refreshed" => Some(EventKind::DayRefreshed),
            "language.changed" => Some(EventKind::LanguageChanged),
            _ => None,
        }
    }
}

/// Where a command is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Window {
    Flyout,
    Week,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDecl {
    pub name: String,
    pub title: Localized,
    pub windows: BTreeSet<Window>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub id: PluginId,
    pub name: Localized,
    pub description: Localized,
    pub version: Version,
    /// The api minor the plugin needs; the major is always [`API_MAJOR`].
    pub api_minor: u32,
    pub memory: u32,
    pub memory_min: u32,
    /// At least one; the first is the fallback.
    pub locales: Vec<String>,
    pub capabilities: BTreeSet<Capability>,
    pub events: BTreeSet<EventKind>,
    pub commands: Vec<CommandDecl>,
}

impl Manifest {
    /// Reads and checks a manifest. `reserved_titles` are the palette labels of the built-in
    /// commands in every language; a plugin command may not take one, by its title alone or as
    /// the « <plugin>: <title> » the palette shows.
    pub fn parse(bytes: &[u8], reserved_titles: &[&str]) -> Result<Manifest, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::at("", Problem::FileTooLarge));
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| ManifestError::at("", Problem::NotJson(e.to_string())))?;
        let Value::Object(object) = value else {
            return Err(ManifestError::at("", Problem::WrongType(Expected::Object)));
        };
        Reader { object: &object }.manifest(reserved_titles)
    }

    pub fn fallback_locale(&self) -> &str {
        &self.locales[0]
    }

    pub fn display_name(&self, language: Language) -> &str {
        self.name.get(language, self.fallback_locale())
    }

    pub fn command(&self, name: &str) -> Option<&CommandDecl> {
        self.commands.iter().find(|c| c.name == name)
    }

    pub fn has(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// A refused manifest: the field (`commands[0].title`) and the rule it broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub field: String,
    pub problem: Problem,
}

impl ManifestError {
    fn at(field: impl Into<String>, problem: Problem) -> ManifestError {
        ManifestError {
            field: field.into(),
            problem,
        }
    }

    /// « `commands[0].title`: this title is already used by Bientôt ».
    pub fn message(&self, language: Language) -> String {
        let rule = self.problem.message(language);
        if self.field.is_empty() {
            rule
        } else {
            format!("`{}`: {rule}", self.field)
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message(Language::En))
    }
}

impl std::error::Error for ManifestError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    Object,
    String,
    List,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    FileTooLarge,
    NotJson(String),
    WrongType(Expected),
    Missing,
    UnknownKey,
    NotSupportedYet,
    BadId,
    ReservedId,
    BadVersion,
    UnsupportedApi,
    BadSize,
    MemoryTooLarge,
    MemoryMinAboveMemory,
    NoLocales,
    BadLocale,
    DuplicateLocale,
    MissingFallbackText,
    TooLong(usize),
    UnknownCapability,
    UnknownEvent,
    NeedsReadDay,
    TooManyCommands,
    BadCommandName,
    DuplicateCommand,
    BuiltinTitle,
    ShortcutRefused,
    BadWindow,
    NoWindow,
    WasiRefused,
}

impl Problem {
    pub fn message(&self, language: Language) -> String {
        let fr = language == Language::Fr;
        let text = |en: &str, fr_text: &str| {
            if fr {
                fr_text.to_owned()
            } else {
                en.to_owned()
            }
        };
        match self {
            Problem::FileTooLarge => text(
                "the manifest is larger than 64 KiB",
                "le manifeste dépasse 64 Kio",
            ),
            Problem::NotJson(detail) => {
                if fr {
                    format!("le manifeste n’est pas du JSON valide ({detail})")
                } else {
                    format!("the manifest is not valid JSON ({detail})")
                }
            }
            Problem::WrongType(expected) => match (expected, fr) {
                (Expected::Object, false) => "must be an object".to_owned(),
                (Expected::Object, true) => "doit être un objet".to_owned(),
                (Expected::String, false) => "must be a string".to_owned(),
                (Expected::String, true) => "doit être une chaîne".to_owned(),
                (Expected::List, false) => "must be a list".to_owned(),
                (Expected::List, true) => "doit être une liste".to_owned(),
                (Expected::Bool, false) => "must be true or false".to_owned(),
                (Expected::Bool, true) => "doit valoir true ou false".to_owned(),
            },
            Problem::Missing => text("is required", "est obligatoire"),
            Problem::UnknownKey => text("is not a manifest key", "n’est pas une clé du manifeste"),
            Problem::NotSupportedYet => text(
                "is not supported yet by this version of Bientôt",
                "n’est pas encore pris en charge par cette version de Bientôt",
            ),
            Problem::BadId => text(
                "must look like com.example.name: lower-case letters, digits and hyphens, 2 to 5 parts, at most 64 characters",
                "doit ressembler à com.exemple.nom : minuscules, chiffres et tirets, de 2 à 5 parties, 64 caractères au plus",
            ),
            Problem::ReservedId => text(
                "ids starting with dev. are reserved",
                "les identifiants qui commencent par dev. sont réservés",
            ),
            Problem::BadVersion => text(
                "must be a version like 1.2.3",
                "doit être une version comme 1.2.3",
            ),
            Problem::UnsupportedApi => {
                if fr {
                    format!(
                        "cette version de Bientôt prend en charge l’api {API_MAJOR}.0 à {API_MAJOR}.{API_MINOR}"
                    )
                } else {
                    format!(
                        "this version of Bientôt supports api {API_MAJOR}.0 to {API_MAJOR}.{API_MINOR}"
                    )
                }
            }
            Problem::BadSize => text(
                "must be a size like \"4MiB\" or \"512KiB\"",
                "doit être une taille comme \"4MiB\" ou \"512KiB\"",
            ),
            Problem::MemoryTooLarge => text("must be at most 32MiB", "doit valoir 32MiB au plus"),
            Problem::MemoryMinAboveMemory => text(
                "must not be larger than memory",
                "ne doit pas dépasser memory",
            ),
            Problem::NoLocales => text(
                "must list at least one locale",
                "doit lister au moins une langue",
            ),
            Problem::BadLocale => text(
                "must be a lower-case language tag like en, fr or en-gb",
                "doit être une étiquette de langue en minuscules comme en, fr ou en-gb",
            ),
            Problem::DuplicateLocale => text("lists a locale twice", "liste deux fois une langue"),
            Problem::MissingFallbackText => text(
                "must have a text for the first locale",
                "doit avoir un texte pour la première langue",
            ),
            Problem::TooLong(max) => {
                if fr {
                    format!("doit faire {max} caractères au plus")
                } else {
                    format!("must be at most {max} characters")
                }
            }
            Problem::UnknownCapability => text(
                "is not a capability of this version of Bientôt",
                "n’est pas une capacité de cette version de Bientôt",
            ),
            Problem::UnknownEvent => text(
                "is not an event a plugin can ask for",
                "n’est pas un événement qu’un plugin peut demander",
            ),
            Problem::NeedsReadDay => text(
                "day.refreshed needs the capability read:day",
                "day.refreshed demande la capacité read:day",
            ),
            Problem::TooManyCommands => text("at most 8 commands", "8 commandes au plus"),
            Problem::BadCommandName => text(
                "must start with a lower-case letter, then lower-case letters, digits or hyphens, at most 32 characters",
                "doit commencer par une minuscule, puis des minuscules, chiffres ou tirets, 32 caractères au plus",
            ),
            Problem::DuplicateCommand => text(
                "another command has this name",
                "une autre commande porte ce nom",
            ),
            Problem::BuiltinTitle => text(
                "this title is already used by Bientôt",
                "ce titre est déjà utilisé par Bientôt",
            ),
            Problem::ShortcutRefused => text(
                "plugins cannot set keyboard shortcuts",
                "un plugin ne peut pas définir de raccourci clavier",
            ),
            Problem::BadWindow => text("must be flyout or week", "doit valoir flyout ou week"),
            Problem::NoWindow => text(
                "must list flyout, week or both",
                "doit lister flyout, week ou les deux",
            ),
            Problem::WasiRefused => text(
                "must be false: plugins do not get WASI",
                "doit valoir false : les plugins n’ont pas accès à WASI",
            ),
        }
    }
}

/// Keys of later steps: refused with « not supported yet » rather than « unknown ».
const LATER_KEYS: &[&str] = &[
    "settings",
    "storage",
    "timers",
    "net",
    "secrets",
    "slots",
    "automations",
    "theme",
    "themes",
    "tray",
    "notifications",
    "reminders",
    "sources",
    "pages",
    "export",
];

const KEYS: &[&str] = &[
    "$schema",
    "id",
    "name",
    "description",
    "version",
    "api",
    "memory",
    "memory_min",
    "locales",
    "capabilities",
    "events",
    "commands",
    "wasi",
];

struct Reader<'a> {
    object: &'a Map<String, Value>,
}

impl Reader<'_> {
    fn manifest(&self, reserved_titles: &[&str]) -> Result<Manifest, ManifestError> {
        check_keys(self.object, "", KEYS)?;
        if let Some(wasi) = self.object.get("wasi") {
            match wasi {
                Value::Bool(false) => {}
                Value::Bool(true) => return Err(ManifestError::at("wasi", Problem::WasiRefused)),
                _ => {
                    return Err(ManifestError::at(
                        "wasi",
                        Problem::WrongType(Expected::Bool),
                    ));
                }
            }
        }
        if self
            .object
            .get("$schema")
            .is_some_and(|schema| !schema.is_string())
        {
            return Err(ManifestError::at(
                "$schema",
                Problem::WrongType(Expected::String),
            ));
        }

        let id_text = required_string(self.object, "id")?;
        let id = PluginId::parse(id_text).ok_or_else(|| ManifestError::at("id", Problem::BadId))?;
        if id.is_reserved() {
            return Err(ManifestError::at("id", Problem::ReservedId));
        }
        let version = Version::parse(required_string(self.object, "version")?)
            .ok_or_else(|| ManifestError::at("version", Problem::BadVersion))?;
        let api = Version::parse(required_string(self.object, "api")?)
            .ok_or_else(|| ManifestError::at("api", Problem::BadVersion))?;
        if api.major != API_MAJOR || api.minor > API_MINOR {
            return Err(ManifestError::at("api", Problem::UnsupportedApi));
        }

        let locales = self.locales()?;
        let fallback = locales[0].as_str();
        let name = localized(
            self.object.get("name"),
            "name",
            fallback,
            MAX_NAME_CHARS,
            true,
        )?;
        let description = localized(
            self.object.get("description"),
            "description",
            fallback,
            MAX_DESCRIPTION_CHARS,
            false,
        )?;

        let memory = optional_size(self.object, "memory")?.unwrap_or(DEFAULT_MEMORY);
        if memory > MAX_MEMORY {
            return Err(ManifestError::at("memory", Problem::MemoryTooLarge));
        }
        let memory_min =
            optional_size(self.object, "memory_min")?.unwrap_or(DEFAULT_MEMORY_MIN.min(memory));
        if memory_min > memory {
            return Err(ManifestError::at(
                "memory_min",
                Problem::MemoryMinAboveMemory,
            ));
        }

        let capabilities = string_set(
            self.object,
            "capabilities",
            Capability::parse,
            Problem::UnknownCapability,
        )?;
        let events = string_set(
            self.object,
            "events",
            EventKind::parse,
            Problem::UnknownEvent,
        )?;
        if events.contains(&EventKind::DayRefreshed) && !capabilities.contains(&Capability::ReadDay)
        {
            return Err(ManifestError::at("events", Problem::NeedsReadDay));
        }
        let commands = self.commands(fallback, &name, reserved_titles)?;

        Ok(Manifest {
            id,
            name,
            description,
            version,
            api_minor: api.minor,
            memory,
            memory_min,
            locales,
            capabilities,
            events,
            commands,
        })
    }

    fn locales(&self) -> Result<Vec<String>, ManifestError> {
        let list = match self.object.get("locales") {
            None => return Err(ManifestError::at("locales", Problem::Missing)),
            Some(Value::Array(list)) => list,
            Some(_) => {
                return Err(ManifestError::at(
                    "locales",
                    Problem::WrongType(Expected::List),
                ));
            }
        };
        if list.is_empty() {
            return Err(ManifestError::at("locales", Problem::NoLocales));
        }
        let mut locales = Vec::with_capacity(list.len());
        for (index, value) in list.iter().enumerate() {
            let field = format!("locales[{index}]");
            let Value::String(tag) = value else {
                return Err(ManifestError::at(
                    field,
                    Problem::WrongType(Expected::String),
                ));
            };
            if !is_locale_tag(tag) {
                return Err(ManifestError::at(field, Problem::BadLocale));
            }
            if locales.contains(tag) {
                return Err(ManifestError::at(field, Problem::DuplicateLocale));
            }
            locales.push(tag.clone());
        }
        Ok(locales)
    }

    fn commands(
        &self,
        fallback: &str,
        plugin: &Localized,
        reserved_titles: &[&str],
    ) -> Result<Vec<CommandDecl>, ManifestError> {
        let list = match self.object.get("commands") {
            None => return Ok(Vec::new()),
            Some(Value::Array(list)) => list,
            Some(_) => {
                return Err(ManifestError::at(
                    "commands",
                    Problem::WrongType(Expected::List),
                ));
            }
        };
        if list.len() > MAX_COMMANDS {
            return Err(ManifestError::at("commands", Problem::TooManyCommands));
        }
        let reserved: Vec<String> = reserved_titles.iter().map(|t| fold(t)).collect();
        let mut commands: Vec<CommandDecl> = Vec::with_capacity(list.len());
        for (index, value) in list.iter().enumerate() {
            let at = |key: &str| format!("commands[{index}].{key}");
            let Value::Object(object) = value else {
                return Err(ManifestError::at(
                    format!("commands[{index}]"),
                    Problem::WrongType(Expected::Object),
                ));
            };
            if object.contains_key("shortcut") {
                return Err(ManifestError::at(at("shortcut"), Problem::ShortcutRefused));
            }
            check_keys(
                object,
                &format!("commands[{index}]."),
                &["name", "title", "windows"],
            )?;
            let name = match object.get("name") {
                None => return Err(ManifestError::at(at("name"), Problem::Missing)),
                Some(Value::String(name)) => name,
                Some(_) => {
                    return Err(ManifestError::at(
                        at("name"),
                        Problem::WrongType(Expected::String),
                    ));
                }
            };
            if !is_command_name(name) {
                return Err(ManifestError::at(at("name"), Problem::BadCommandName));
            }
            if commands.iter().any(|c| &c.name == name) {
                return Err(ManifestError::at(at("name"), Problem::DuplicateCommand));
            }
            let title = localized(
                object.get("title"),
                &at("title"),
                fallback,
                MAX_TITLE_CHARS,
                true,
            )?;
            let shown = title.values().flat_map(|title| {
                plugin
                    .values()
                    .map(move |plugin| format!("{plugin}: {title}"))
                    .chain([title.to_owned()])
            });
            if shown
                .map(|label| fold(&label))
                .any(|label| reserved.contains(&label))
            {
                return Err(ManifestError::at(at("title"), Problem::BuiltinTitle));
            }
            let windows = match object.get("windows") {
                None => BTreeSet::from([Window::Flyout, Window::Week]),
                Some(Value::Array(list)) => {
                    let mut windows = BTreeSet::new();
                    for (w, value) in list.iter().enumerate() {
                        let window = match value.as_str() {
                            Some("flyout") => Window::Flyout,
                            Some("week") => Window::Week,
                            _ => {
                                return Err(ManifestError::at(
                                    format!("commands[{index}].windows[{w}]"),
                                    Problem::BadWindow,
                                ));
                            }
                        };
                        windows.insert(window);
                    }
                    if windows.is_empty() {
                        return Err(ManifestError::at(at("windows"), Problem::NoWindow));
                    }
                    windows
                }
                Some(_) => {
                    return Err(ManifestError::at(
                        at("windows"),
                        Problem::WrongType(Expected::List),
                    ));
                }
            };
            commands.push(CommandDecl {
                name: name.clone(),
                title,
                windows,
            });
        }
        Ok(commands)
    }
}

fn check_keys(
    object: &Map<String, Value>,
    prefix: &str,
    allowed: &[&str],
) -> Result<(), ManifestError> {
    for key in object.keys() {
        if allowed.contains(&key.as_str()) {
            continue;
        }
        let problem = if LATER_KEYS.contains(&key.as_str()) {
            Problem::NotSupportedYet
        } else {
            Problem::UnknownKey
        };
        return Err(ManifestError::at(format!("{prefix}{key}"), problem));
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, ManifestError> {
    match object.get(key) {
        None => Err(ManifestError::at(key, Problem::Missing)),
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(ManifestError::at(key, Problem::WrongType(Expected::String))),
    }
}

fn localized(
    value: Option<&Value>,
    field: &str,
    fallback: &str,
    max_chars: usize,
    required: bool,
) -> Result<Localized, ManifestError> {
    let object = match value {
        None if required => return Err(ManifestError::at(field, Problem::Missing)),
        None => return Ok(Localized::default()),
        Some(Value::Object(object)) => object,
        Some(_) => {
            return Err(ManifestError::at(
                field,
                Problem::WrongType(Expected::Object),
            ));
        }
    };
    let mut texts = BTreeMap::new();
    for (tag, text) in object {
        let at = format!("{field}.{tag}");
        if !is_locale_tag(tag) {
            return Err(ManifestError::at(at, Problem::BadLocale));
        }
        let Value::String(text) = text else {
            return Err(ManifestError::at(at, Problem::WrongType(Expected::String)));
        };
        if text.chars().count() > max_chars {
            return Err(ManifestError::at(at, Problem::TooLong(max_chars)));
        }
        texts.insert(tag.clone(), text.clone());
    }
    let has_fallback = texts.get(fallback).is_some_and(|t| !t.trim().is_empty());
    if !has_fallback && (required || !texts.is_empty()) {
        return Err(ManifestError::at(field, Problem::MissingFallbackText));
    }
    Ok(Localized(texts))
}

fn optional_size(object: &Map<String, Value>, key: &str) -> Result<Option<u32>, ManifestError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => parse_size(text)
            .map(Some)
            .ok_or_else(|| ManifestError::at(key, Problem::BadSize)),
        Some(_) => Err(ManifestError::at(key, Problem::BadSize)),
    }
}

/// `"4MiB"`, `"512KiB"`: a whole number then `KiB` or `MiB`.
pub fn parse_size(text: &str) -> Option<u32> {
    let (number, shift) = if let Some(n) = text.strip_suffix("MiB") {
        (n, 20)
    } else {
        (text.strip_suffix("KiB")?, 10)
    };
    if number.is_empty() || number.len() > 6 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: u64 = number.parse().ok()?;
    let bytes = value << shift;
    (bytes > 0 && bytes <= u64::from(u32::MAX)).then_some(bytes as u32)
}

fn string_set<T: Ord>(
    object: &Map<String, Value>,
    key: &str,
    parse: impl Fn(&str) -> Option<T>,
    unknown: Problem,
) -> Result<BTreeSet<T>, ManifestError> {
    let list = match object.get(key) {
        None => return Ok(BTreeSet::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(ManifestError::at(key, Problem::WrongType(Expected::List))),
    };
    let mut set = BTreeSet::new();
    for (index, value) in list.iter().enumerate() {
        let field = format!("{key}[{index}]");
        let Value::String(text) = value else {
            return Err(ManifestError::at(
                field,
                Problem::WrongType(Expected::String),
            ));
        };
        set.insert(parse(text).ok_or_else(|| ManifestError::at(field, unknown.clone()))?);
    }
    Ok(set)
}

/// `en`, `fr`, `en-gb`: lower case, a 2 or 3 letter language and optional subtags.
pub fn is_locale_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or("");
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    tag.len() <= 16
        && parts.all(|p| {
            (2..=8).contains(&p.len())
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn is_command_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && name.len() <= 32
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A title as compared with the built-in ones: lower case, spaces collapsed, including the
/// no-break spaces of French typography, and none before a colon.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" :", ":")
}

#[cfg(test)]
mod tests;
