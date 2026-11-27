//! The notification-area icon Linux draws for a panel of unknown colour, every state at each size
//! the sheet names, on panels of seven desktops' colours: `tray-icons-any-panel.png`, Linux only.
//! Each panel shows the icons at their own pixels, then those of 16 and 22 px four times larger,
//! pixel for pixel. The sheet has no words, so it is the same in every language: each panel is
//! captioned with its colour and the sizes, and its columns are the states in a fixed order,
//! green, then green with a source in trouble, then orange, red and blue alike.

use std::sync::Arc;

use bientot_app::strings::Language;
use bientot_app::theme::spacing::{BASE_04, BASE_08, BASE_12, BASE_16};
use bientot_app::theme::{Theme, TypeStyled as _, text};
use bientot_app::tray_art::{self, Taskbar};
use bientot_app::workday::{TrayColour, TrayIcon};
use gpui::{
    AppContext as _, Context, HeadlessAppContext, IntoElement, ParentElement as _, Render,
    RenderImage, Styled as _, Window, div, img, px, rgb, size,
};

use super::capture;
use super::platform::{Platform, Shots};

/// The panels' colours: GNOME's two, Breeze light and dark, Mint-Y dark and light, and grey.
const PANELS: [u32; 7] = [
    0x000000, 0x1d1d1d, 0xeff0f1, 0x31363b, 0x2f2f2f, 0xf2f2f2, 0x808080,
];
/// The sizes a Linux panel asks for (`bientot_linux::tray::ICON_SIZES`).
const SIZES: [u32; 5] = [16, 22, 24, 32, 48];
/// The sizes drawn four times larger.
const ZOOMED: [u32; 2] = [16, 22];
const ZOOM: u32 = 4;
const COLOURS: [TrayColour; 4] = [
    TrayColour::Green,
    TrayColour::Orange,
    TrayColour::Red,
    TrayColour::Blue,
];
/// The width of a state's column, in pixels of the layout.
const COLUMN: f32 = 88.;

pub fn capture_icons(
    cx: &mut HeadlessAppContext,
    _language: Language,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    if cx.update(|cx| cx.global::<Shots>().platform) != Platform::Linux {
        return Ok(());
    }
    let sheet = Sheet::new()?;
    let window = cx.open_window(size(px(860.), px(960.)), |_, cx| cx.new(|_| sheet))?;
    capture(cx, window.into(), &output.join("tray-icons-any-panel.png"))
}

/// Images of the icon, each with its side in pixels.
type Drawn = Vec<(u32, Arc<RenderImage>)>;

/// A state of the icon, drawn at each size and zoom.
struct State {
    drawn: Drawn,
    zoomed: Drawn,
}

struct Sheet {
    states: Vec<State>,
}

impl Sheet {
    fn new() -> anyhow::Result<Self> {
        let mut states = Vec::new();
        for colour in COLOURS {
            for degraded in [false, true] {
                let icon = TrayIcon { colour, degraded };
                let draw = |sizes: &[u32], zoom| {
                    sizes
                        .iter()
                        .map(|&size| {
                            let rgba = tray_art::rgba(icon, size, Taskbar::Unknown)?;
                            Ok((size * zoom, image(&rgba, size, zoom)))
                        })
                        .collect::<anyhow::Result<Drawn>>()
                };
                states.push(State {
                    drawn: draw(&SIZES, 1)?,
                    zoomed: draw(&ZOOMED, ZOOM)?,
                });
            }
        }
        Ok(Self { states })
    }
}

/// `rgba`, an icon of `size` pixels, each pixel made a square of `zoom` pixels, in the BGRA order
/// GPUI draws.
fn image(rgba: &[u8], size: u32, zoom: u32) -> Arc<RenderImage> {
    let buffer = image::RgbaImage::from_fn(size * zoom, size * zoom, |x, y| {
        let at = (((y / zoom) * size + x / zoom) * 4) as usize;
        image::Rgba([rgba[at + 2], rgba[at + 1], rgba[at], rgba[at + 3]])
    });
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// A panel's caption: its colour, the sizes at their own pixels, then the zoomed ones.
fn caption(colour: u32) -> String {
    let list = |sizes: &[u32]| {
        sizes
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" · ")
    };
    format!(
        "#{colour:06x}    {} px    {ZOOM}× {} px",
        list(&SIZES),
        list(&ZOOMED)
    )
}

impl Render for Sheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        // One pixel of an image to one pixel of the screenshot, which GPUI then samples exactly.
        let scale = window.scale_factor();
        let row = |images: fn(&State) -> &Drawn| {
            div()
                .flex()
                .gap(BASE_12)
                .p(BASE_08)
                .children(self.states.iter().map(move |state| {
                    div()
                        .w(px(COLUMN))
                        .flex()
                        .items_end()
                        .gap(BASE_04)
                        .children(images(state).iter().map(|(pixels, image)| {
                            let side = px(*pixels as f32 / scale);
                            img(image.clone()).w(side).h(side)
                        }))
                }))
        };
        div()
            .size_full()
            .p(BASE_16)
            .flex()
            .flex_col()
            .gap(BASE_12)
            .bg(theme.background)
            .text_color(theme.text)
            .children(PANELS.iter().map(|colour| {
                div()
                    .flex()
                    .flex_col()
                    .gap(BASE_04)
                    .child(div().type_style(text::LABEL_SM).child(caption(*colour)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .bg(rgb(*colour))
                            .child(row(|state| &state.drawn))
                            .child(row(|state| &state.zoomed)),
                    )
            }))
    }
}
