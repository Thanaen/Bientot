//! « À propos » in « Général »: the version and the day it was published, then how Bientôt was
//! installed where the system tells; then « Diagnostic », where the backends tell.

use gpui::{Role, SharedString, div, prelude::*};

use super::{group, row_line, row_text};
use crate::settings::{AboutView, DiagnosticView};
use crate::theme::Theme;
use crate::theme::spacing::*;

/// « À propos »: the version where a row has its control, the day it was published as its
/// description, and no description when the build does not know it. A Flatpak or an AppImage
/// names itself in a row under it.
pub(super) fn section(about: AboutView, theme: &Theme) -> impl IntoElement {
    let text = row_text(
        about.title,
        theme.text,
        about.released.map(IntoElement::into_any_element),
        theme.text_muted,
    );
    let version = div()
        .py(BASE_12)
        .when(about.install.is_some(), |row| {
            row.border_b_1().border_color(theme.border_variant)
        })
        .child(row_line(text, value("version", about.label, about.version)));
    let install = about.install.map(|install| {
        let text = row_text(install.title, theme.text, None, theme.text_muted);
        let label = format!("{} {}", install.title, install.value);
        div()
            .py(BASE_12)
            .child(row_line(text, value("install-kind", label, install.value)))
            .into_any_element()
    });
    group(
        "about",
        Some(about.heading),
        std::iter::once(version.into_any_element()).chain(install),
        theme,
    )
}

/// « Diagnostic »: the idle backend's name at the end of its row, and the sentence that says the
/// desktop does not report locks under its title.
pub(super) fn diagnostic_section(diagnostic: DiagnosticView, theme: &Theme) -> impl IntoElement {
    let activity = diagnostic.activity.map(|activity| {
        let text = row_text(activity.title, theme.text, None, theme.text_muted);
        let label = format!("{} {}", activity.title, activity.value);
        div()
            .py(BASE_12)
            .when(diagnostic.locks.is_some(), |row| {
                row.border_b_1().border_color(theme.border_variant)
            })
            .child(row_line(text, value("idle-backend", label, activity.value)))
            .into_any_element()
    });
    let locks = diagnostic.locks.map(|locks| {
        div()
            .py(BASE_12)
            .child(row_text(
                locks.title,
                theme.text,
                Some(locks.description.into_any_element()),
                theme.text_muted,
            ))
            .into_any_element()
    });
    group(
        "diagnostic",
        Some(diagnostic.heading),
        activity.into_iter().chain(locks),
        theme,
    )
}

/// The value at the end of a row, which a screen reader names with its row's title: `label`.
fn value(
    id: &'static str,
    label: impl Into<SharedString>,
    value: &'static str,
) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Label)
        // AccessKit names a Label from its value, not its label.
        .aria_value(label)
        .flex_none()
        .whitespace_nowrap()
        .child(value)
}
