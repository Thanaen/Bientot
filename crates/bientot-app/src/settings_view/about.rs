//! « À propos » in « Général »: the version and the day it was published.

use gpui::{Role, div, prelude::*};

use super::{group, row_line, row_text};
use crate::settings::AboutView;
use crate::theme::Theme;
use crate::theme::spacing::*;

/// « À propos »: the version where a row has its control, the day it was published as its
/// description, and no description when the build does not know it.
pub(super) fn section(about: AboutView, theme: &Theme) -> impl IntoElement {
    let version = div()
        .id("version")
        .role(Role::Label)
        .aria_label(about.label)
        .flex_none()
        .whitespace_nowrap()
        .child(about.version);
    let text = row_text(
        about.title,
        theme.text,
        about.released.map(IntoElement::into_any_element),
        theme.text_muted,
    );
    let row = div().py(BASE_12).child(row_line(text, version));
    group(
        "about",
        Some(about.heading),
        [row.into_any_element()],
        theme,
    )
}
