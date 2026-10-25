//! The version resource of `worktime.exe`: Task Manager, the Startup apps page and the file's
//! Properties name the app from its `FileDescription` and `ProductName`, not from the exe's name,
//! which stays `worktime.exe` for the Run value and Velopack's `mainExe`.

const NAME: &str = "Bientôt";
const COMPANY: &str = "Thanaen";
pub const COPYRIGHT: &str = "Copyright (c) 2026 Thanaen";
const ORIGINAL_FILENAME: &str = "worktime.exe";

/// The `.rc` source for `version`, `0.2.0` or a release's `0.2.0-alpha.<commit count>`.
pub fn rc(version: &str) -> Result<String, String> {
    let [major, minor, patch, build] = numbers(version)?;
    let numeric = format!("{major},{minor},{patch},{build}");
    let strings = [
        ("CompanyName", COMPANY),
        ("FileDescription", NAME),
        ("FileVersion", version),
        ("InternalName", "worktime"),
        ("LegalCopyright", COPYRIGHT),
        ("OriginalFilename", ORIGINAL_FILENAME),
        ("ProductName", NAME),
        ("ProductVersion", version),
    ];
    let values: String = strings
        .iter()
        .map(|(key, value)| format!("      VALUE \"{key}\", {}\n", wide(value)))
        .collect();
    // French (0x40C) in UTF-16 (1200), for a Windows NT (0x40004) application (0x1).
    Ok(format!(
        r#"1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040C04B0"
    BEGIN
{values}    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x40C, 1200
  END
END
"#
    ))
}

/// The four numbers of `FILEVERSION`: the release's commit count is the fourth, so that two
/// alpha builds of the same base version still compare in order.
fn numbers(version: &str) -> Result<[u16; 4], String> {
    let invalid = || format!("{version:?} is not a version the resource can carry");
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let mut parts = core.split('.').map(str::parse::<u16>);
    let (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    let build = match pre {
        None => 0,
        Some(pre) => pre
            .rsplit('.')
            .next()
            .and_then(|last| last.parse().ok())
            .ok_or_else(invalid)?,
    };
    Ok([major, minor, patch, build])
}

/// A wide string literal with every character outside printable ASCII as a `\x` escape: RC.EXE
/// reads a narrow literal in the build machine's code page, and llvm-rc in the one it is given.
fn wide(value: &str) -> String {
    let mut literal = String::from("L\"");
    for unit in value.encode_utf16() {
        match char::from_u32(unit.into()) {
            Some(c) if c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\') => {
                literal.push(c)
            }
            _ => literal.push_str(&format!("\\x{unit:04X}")),
        }
    }
    literal.push('"');
    literal
}
