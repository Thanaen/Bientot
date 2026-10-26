//! The notification-area icon, drawn at the exact size the taskbar shows it. Windows shrinks a
//! larger icon without smoothing, so each size gets its own drawing, its strokes on whole pixels.

use std::fmt::Write as _;

use resvg::{tiny_skia, usvg};

use crate::workday::{TrayColour, TrayIcon};

/// The taskbar's theme, which the badge of a source in trouble contrasts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taskbar {
    Dark,
    Light,
}

/// Each hue passes 3:1 against the white symbol and both taskbars.
fn fill(colour: TrayColour) -> &'static str {
    match colour {
        TrayColour::Green => "#15803D",
        TrayColour::Orange => "#D9590B",
        TrayColour::Red => "#DC2626",
        TrayColour::Blue => "#2563EB",
    }
}

/// The symbols as stroked paths in a box from -1 to 1, y down: a check on track, a cross past the
/// limit, and an arrow out once the target is reached. The orange "!" is drawn on the pixel grid.
fn path(colour: TrayColour) -> &'static [(f32, f32)] {
    match colour {
        TrayColour::Green => &[(-0.58, 0.02), (-0.16, 0.44), (0.62, -0.4)],
        TrayColour::Red => &[
            (-0.46, -0.46),
            (0.46, 0.46),
            (f32::NAN, 0.),
            (0.46, -0.46),
            (-0.46, 0.46),
        ],
        TrayColour::Blue => &[
            (-0.62, 0.),
            (0.56, 0.),
            (f32::NAN, 0.),
            (0.06, -0.52),
            (0.6, 0.),
            (0.06, 0.52),
        ],
        TrayColour::Orange => &[],
    }
}

/// The icon at `size` pixels as SVG: a full disc in the day's colour with its white symbol, and
/// for a source in trouble a dot in the taskbar's text colour, cut out of the bottom-right corner.
pub fn svg(icon: TrayIcon, size: u32, taskbar: Taskbar) -> String {
    let s = size as f32;
    let stroke = match size {
        16 | 20 => 2,
        24 | 28 => 3,
        32 => 4,
        _ => (size as f32 / 8.).round() as u32,
    };
    // The symbol moves away from the badge so the badge does not hide it.
    let shift = if icon.degraded {
        (s / 24.).round().max(1.)
    } else {
        0.
    };
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">"#
    );
    let badge = icon.degraded.then(|| {
        let diameter = match size {
            16 => 5.,
            20 => 6.,
            24 => 8.,
            32 => 10.,
            _ => (s * 0.32).round(),
        };
        let gap = (s / 24.).round().max(1.);
        (s - diameter / 2., diameter / 2., gap)
    });
    if let Some((centre, radius, gap)) = badge {
        let _ = write!(
            out,
            r##"<defs><mask id="badge"><rect width="{size}" height="{size}" fill="#fff"/><circle cx="{centre}" cy="{centre}" r="{}" fill="#000"/></mask></defs><g mask="url(#badge)">"##,
            radius + gap
        );
    } else {
        out.push_str("<g>");
    }
    let _ = write!(
        out,
        r#"<circle cx="{0}" cy="{0}" r="{0}" fill="{1}"/>"#,
        s / 2.,
        fill(icon.colour)
    );
    let rounding = if size < 24 { 0. } else { stroke as f32 / 4. };
    if icon.colour == TrayColour::Orange {
        let x = (s / 2. - stroke as f32 / 2. + 0.5).floor() - shift;
        let (bar_top, bar_bottom, dot_top) = match size {
            16 => (3., 10., 11.),
            20 => (4., 12., 14.),
            24 => (5., 15., 17.),
            32 => (6., 19., 22.),
            _ => {
                let bottom = (s * 0.6).round();
                ((s * 0.2).round(), bottom, bottom + (s / 12.).round())
            }
        };
        for (top, height) in [(bar_top, bar_bottom - bar_top), (dot_top, stroke as f32)] {
            let _ = write!(
                out,
                r##"<rect x="{x}" y="{}" width="{stroke}" height="{height}" rx="{rounding}" fill="#fff"/>"##,
                top - shift
            );
        }
    } else {
        // An odd stroke is centred on a pixel's middle, so its edges fall on whole pixels.
        let centre = s / 2. + if stroke % 2 == 1 { 0.5 } else { 0. } - shift;
        let scale = s / 2.
            * if icon.colour == TrayColour::Red {
                0.62
            } else {
                0.66
            };
        let mut d = String::new();
        let mut command = 'M';
        for &(x, y) in path(icon.colour) {
            if x.is_nan() {
                command = 'M';
                continue;
            }
            let _ = write!(
                d,
                "{command}{:.3} {:.3} ",
                centre + x * scale,
                centre + y * scale
            );
            command = 'L';
        }
        let _ = write!(
            out,
            r##"<path d="{}" fill="none" stroke="#fff" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round"/>"##,
            d.trim_end()
        );
    }
    out.push_str("</g>");
    if let Some((centre, radius, _)) = badge {
        let text = match taskbar {
            Taskbar::Dark => "#FFFFFF",
            Taskbar::Light => "#1A1A1A",
        };
        let _ = write!(
            out,
            r#"<circle cx="{centre}" cy="{centre}" r="{radius}" fill="{text}"/>"#
        );
    }
    out.push_str("</svg>");
    out
}

/// The icon at `size` pixels as straight (not premultiplied) RGBA rows, as `tray-icon` takes it.
pub fn rgba(icon: TrayIcon, size: u32, taskbar: Taskbar) -> anyhow::Result<Vec<u8>> {
    let tree = usvg::Tree::from_str(&svg(icon, size, taskbar), &usvg::Options::default())?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| anyhow::anyhow!("cannot draw a {size} px tray icon"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect())
}

#[cfg(test)]
mod tests;
