//! Fonts and icons compiled into the binary, so WorkTime never depends on what Windows has installed.

use std::borrow::Cow;

use gpui::{App, AssetSource, SharedString};

pub const FONT_FAMILY: &str = "IBM Plex Sans";
/// The figures of the week's timeline and of what the user edits.
pub const MONO_FONT_FAMILY: &str = "IBM Plex Mono";

const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
];

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "activity",
    "bell",
    "braces",
    "calendar",
    "calendar-off",
    "check",
    "chevron-down",
    "chevron-right",
    "chevron-up",
    "circle-alert",
    "circle-arrow-up",
    "circle-check",
    "circle-x",
    "clock",
    "database",
    "flag",
    "house",
    "info",
    "loader-circle",
    "pause",
    "minus",
    "pencil",
    "play",
    "refresh-cw",
    "repeat",
    "search",
    "shield",
    "sigma",
    "sliders-horizontal",
    "square",
    "triangle-alert",
    "x",
];

/// Lucide icons used by WorkTime, addressed by their file name under `icons/`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

pub fn load_fonts(cx: &App) -> anyhow::Result<()> {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|font| Cow::Borrowed(*font)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_icons_by_path() {
        assert!(Assets.load("icons/pause.svg").unwrap().is_some());
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
    }
}
