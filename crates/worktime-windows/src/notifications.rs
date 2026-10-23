//! Toast notifications, ported from `Notifications/WindowsNotificationService.cs`. The .NET app
//! shows them through the Windows App SDK's `AppNotificationManager`; the Rust app has no such
//! runtime, so on Windows [`Toasts`] shows the same toast through WinRT's
//! `ToastNotificationManager`. When to notify and the wording stay with the caller.

#[cfg(windows)]
mod native;
pub mod portable;

#[cfg(windows)]
pub use native::{Sender, Toasts};

/// The `ToastActivatorCLSID` of the MSIX manifest, `packaging/msix/AppxManifest.xml`: the COM
/// class through which Windows hands the package the clicks on its toasts.
pub const PACKAGED_ACTIVATOR: u128 = 0x6cb667aa_56d8_4f29_b21c_7d6f329b4463;

/// The argument WorkTime's COM server, the manifest's or the portable build's, starts it with
/// when no running WorkTime has registered the activator, so that it registers it and receives
/// the click.
pub const ACTIVATOR_ARGUMENT: &str = "--toast-activated";

/// The flag COM adds after a local server's own arguments when it starts one.
pub const COM_SERVER_ARGUMENT: &str = "-Embedding";

/// One toast: two lines of text, and the delivery id the receipts recorded it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationRequest {
    pub delivery_id: String,
    pub title: String,
    pub body: String,
}

/// The toast XML `AppNotificationBuilder` writes for `.AddText(title).AddText(body)
/// .AddArgument("deliveryId", id)`: the argument goes in `launch`, which a click hands back.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the toasts build it outside the tests")
)]
fn toast_xml(request: &NotificationRequest) -> String {
    format!(
        r#"<toast launch="{}"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        escape_xml(&launch_arguments(&request.delivery_id)),
        escape_xml(&request.title),
        escape_xml(&request.body),
    )
}

/// The `launch` arguments of the toast, which Windows hands back unchanged when it is clicked.
fn launch_arguments(delivery_id: &str) -> String {
    format!("deliveryId={}", escape_argument(delivery_id))
}

/// The delivery id in the arguments a click hands back. A click on a toast without one, which
/// WorkTime never shows, still activates the app.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the toasts read it outside the tests")
)]
fn delivery_id(arguments: &str) -> Option<String> {
    arguments
        .split(';')
        .find_map(|argument| match argument.split_once('=') {
            Some(("deliveryId", value)) => Some(unescape_argument(value)),
            _ => None,
        })
}

/// Arguments are `key=value` pairs joined by `;`, so a value escapes those and `%` as the
/// builder does.
fn escape_argument(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace(';', "%3B")
        .replace('=', "%3D")
}

/// Undoes `escape_argument` in one pass, so a decoded `%` never starts another escape.
fn unescape_argument(value: &str) -> String {
    let mut unescaped = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        unescaped.push_str(&rest[..start]);
        rest = &rest[start..];
        let character = match rest.get(..3) {
            Some("%25") => '%',
            Some("%3B") => ';',
            Some("%3D") => '=',
            _ => {
                unescaped.push('%');
                rest = &rest[1..];
                continue;
            }
        };
        unescaped.push(character);
        rest = &rest[3..];
    }
    unescaped.push_str(rest);
    unescaped
}

fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests;
