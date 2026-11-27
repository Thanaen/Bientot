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
    /// A panel whose colour the system does not tell, as on Linux: the icon carries its own
    /// contrast, a white rim around the disc and a dark badge in a white ring.
    Unknown,
}

/// The badge's dark colour: the text of a light taskbar.
const INK: &str = "#1A1A1A";

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

/// The icon at `size` pixels as SVG, for `taskbar`.
pub fn svg(icon: TrayIcon, size: u32, taskbar: Taskbar) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">"#
    );
    match taskbar {
        Taskbar::Dark => cut_out(&mut out, icon, size, "#FFFFFF"),
        Taskbar::Light => cut_out(&mut out, icon, size, INK),
        Taskbar::Unknown => rimmed(&mut out, icon, size),
    }
    out.push_str("</svg>");
    out
}

/// The icon for a taskbar whose text is `text`: a full disc in the day's colour with its white
/// symbol, and for a source in trouble a dot in `text`, cut out of the bottom-right corner.
fn cut_out(out: &mut String, icon: TrayIcon, size: u32, text: &str) {
    let s = size as f32;
    // The badge stays small enough to keep clear of the centred symbol at every size.
    let badge = icon.degraded.then(|| {
        let diameter = (s / 4.).round();
        let gap = if size >= 32 {
            2.
        } else {
            (s / 24.).round().max(1.)
        };
        (s - diameter / 2., diameter / 2., gap)
    });
    if let Some((centre, radius, gap)) = badge {
        let cut = if size == 16 {
            dot16(12, 11, 4)
        } else {
            format!(
                r#"<circle cx="{centre}" cy="{centre}" r="{}""#,
                radius + gap
            )
        };
        let _ = write!(
            out,
            r##"<defs><mask id="badge"><rect width="{size}" height="{size}" fill="#fff"/>{cut} fill="#000"/></mask></defs><g mask="url(#badge)">"##
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
    symbol(out, icon.colour, size);
    out.push_str("</g>");
    if let Some((centre, radius, _)) = badge {
        let dot = if size == 16 {
            dot16(13, 12, 2)
        } else {
            format!(r#"<circle cx="{centre}" cy="{centre}" r="{radius}""#)
        };
        let _ = write!(out, r#"{dot} fill="{text}"/>"#);
    }
}

/// The white symbol of `colour`, centred in an icon of `size` pixels.
fn symbol(out: &mut String, colour: TrayColour, size: u32) {
    let s = size as f32;
    let stroke = match size {
        16 | 20 => 2,
        24 | 28 => 3,
        32 => 4,
        _ => (size as f32 / 8.).round() as u32,
    };
    let rounding = if size < 24 { 0. } else { stroke as f32 / 4. };
    if colour == TrayColour::Orange {
        let x = (s / 2. - stroke as f32 / 2. + 0.5).floor();
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
                top
            );
        }
    } else {
        // An odd stroke is centred on a pixel's middle, so its edges fall on whole pixels.
        let centre = s / 2. + if stroke % 2 == 1 { 0.5 } else { 0. };
        let scale = s / 2.
            * if colour == TrayColour::Red {
                0.62
            } else {
                0.66
            };
        let mut d = String::new();
        let mut command = 'M';
        for &(x, y) in path(colour) {
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
}

/// The icon for a panel of unknown colour: the day's disc inset by a white rim, which parts it
/// from a dark panel as its own edge parts it from a light one, and for a source in trouble a
/// dark dot in a white ring, which reads on both, tangent to the bottom and right edges.
///
/// The disc is drawn on whole pixels, so each pixel of the rim is white or the day's colour, never
/// a pale blend of the two; only the outer edge, against the panel, is smoothed. A rim of 1.5 px
/// keeps a whole white pixel at every angle, where 1 px leaves only blends at the diagonals.
fn rimmed(out: &mut String, icon: TrayIcon, size: u32) {
    let s = size as f32;
    let half = s / 2.;
    let rim = if size >= 32 { 2. } else { 1.5 };
    let badge = icon.degraded.then(|| {
        // At 16 and 24 px a smaller ring than the rule's keeps a pixel of the disc between it and
        // the dot of the orange mark.
        let (ring, dot) = match size {
            16 => (3., 2.),
            24 => (4., 2.),
            _ => {
                let dot = (s / 8.).floor();
                (dot + 2., dot)
            }
        };
        (s - ring, ring, dot)
    });
    let _ = write!(
        out,
        r##"<circle cx="{half}" cy="{half}" r="{half}" fill="#FFFFFF"/>"##
    );
    let mut disc = Pixels::circle(size, half, half, half - rim);
    let white = badge.map(|(centre, ring, _)| {
        // Under the disc, so its smoothed edge only meets the panel.
        let _ = write!(
            out,
            r##"<circle cx="{centre}" cy="{centre}" r="{ring}" fill="#FFFFFF"/>"##
        );
        disc.give_way(&Pixels::circle(size, centre, centre, ring))
    });
    let colour = fill(icon.colour);
    let _ = write!(out, r#"<path d="{}" fill="{colour}"/>"#, disc.path());
    symbol(out, icon.colour, size);
    if let Some(white) = white {
        // Drawn again over the symbol, the ring and the disc's pixels around it keep a whole pixel
        // of the disc between the two; the symbol keeps every other pixel.
        let mut around = white.grown();
        for (pixel, in_disc) in around.set.iter_mut().zip(&disc.set) {
            *pixel &= in_disc;
        }
        let _ = write!(
            out,
            r##"<path d="{}" fill="#FFFFFF"/><path d="{}" fill="{colour}"/>"##,
            white.path(),
            around.path()
        );
    }
    if let Some((centre, _, dot)) = badge {
        let _ = write!(
            out,
            r#"<circle cx="{centre}" cy="{centre}" r="{dot}" fill="{INK}"/>"#
        );
    }
}

/// A set of whole pixels in an icon of `size` pixels, row by row.
struct Pixels {
    size: u32,
    set: Vec<bool>,
}

impl Pixels {
    /// The pixels whose centre lies inside the circle of `radius` around (`cx`, `cy`).
    fn circle(size: u32, cx: f32, cy: f32, radius: f32) -> Self {
        let set = (0..size * size)
            .map(|at| {
                let (x, y) = ((at % size) as f32 + 0.5, (at / size) as f32 + 0.5);
                (x - cx).hypot(y - cy) < radius
            })
            .collect();
        Self { size, set }
    }

    fn has(&self, (x, y): (i64, i64)) -> bool {
        let size = i64::from(self.size);
        (0..size).contains(&x) && (0..size).contains(&y) && self.set[(y * size + x) as usize]
    }

    /// Takes the pixels of `other` out of these, and with them every sliver it would leave: a run
    /// of one or two pixels, across or down, that touches a taken pixel. Gives back what was taken.
    fn give_way(&mut self, other: &Self) -> Self {
        let mut taken = Self {
            size: self.size,
            set: self
                .set
                .iter()
                .zip(&other.set)
                .map(|(a, b)| *a && *b)
                .collect(),
        };
        let size = i64::from(self.size);
        loop {
            for (pixel, gone) in self.set.iter_mut().zip(&taken.set) {
                *pixel &= !gone;
            }
            let mut slivers = vec![false; self.set.len()];
            for across in [true, false] {
                for line in 0..size {
                    let at = |step: i64| {
                        if across { (step, line) } else { (line, step) }
                    };
                    let mut step = 0;
                    while step < size {
                        if !self.has(at(step)) {
                            step += 1;
                            continue;
                        }
                        let start = step;
                        while step < size && self.has(at(step)) {
                            step += 1;
                        }
                        if step - start <= 2 && (taken.has(at(start - 1)) || taken.has(at(step))) {
                            for run in start..step {
                                let (x, y) = at(run);
                                slivers[(y * size + x) as usize] = true;
                            }
                        }
                    }
                }
            }
            if !slivers.contains(&true) {
                return taken;
            }
            for (gone, sliver) in taken.set.iter_mut().zip(slivers) {
                *gone |= sliver;
            }
        }
    }

    /// These pixels and every pixel that touches one, side or corner.
    fn grown(&self) -> Self {
        let size = i64::from(self.size);
        let set = (0..size * size)
            .map(|at| {
                let (x, y) = (at % size, at / size);
                (-1..=1).any(|dy| (-1..=1).any(|dx| self.has((x + dx, y + dy))))
            })
            .collect();
        Self {
            size: self.size,
            set,
        }
    }

    /// As SVG path data: a rectangle for each run of pixels in a row.
    fn path(&self) -> String {
        let size = i64::from(self.size);
        let mut d = String::new();
        for y in 0..size {
            let mut x = 0;
            while x < size {
                if !self.has((x, y)) {
                    x += 1;
                    continue;
                }
                let start = x;
                while self.has((x, y)) {
                    x += 1;
                }
                let _ = write!(d, "M{start} {y}h{}v1h-{}z", x - start, x - start);
            }
        }
        d
    }
}

/// A round dot on whole pixels for the 16 px icon, where a smoothed 4 px circle reads as a smudge:
/// a square `side + 2` pixels wide without its corner pixels, its top edge `side` pixels long from
/// (`x`, `y`). The element is left open for the caller's fill.
fn dot16(x: u32, y: u32, side: u32) -> String {
    format!(r#"<path d="M{x} {y}h{side}v1h1v{side}h-1v1h-{side}v-1h-1v-{side}h1z""#)
}

/// The icon at `size` pixels as straight (not premultiplied) RGBA rows, as `tray-icon` takes it.
pub fn rgba(icon: TrayIcon, size: u32, taskbar: Taskbar) -> anyhow::Result<Vec<u8>> {
    let tree = usvg::Tree::from_str(&svg(icon, size, taskbar), &usvg::Options::default())?;
    // Where a shape reaches the pixmap's left or top edge, tiny-skia covers its edge pixels up to
    // a quarter more than their mirror on the other side, so a disc looks lopsided. The rimmed
    // icon is drawn with a margin, then cut out; the others keep the pixels they have always had.
    let margin = if taskbar == Taskbar::Unknown { 1 } else { 0 };
    let mut pixmap = tiny_skia::Pixmap::new(size + 2 * margin, size + 2 * margin)
        .ok_or_else(|| anyhow::anyhow!("cannot draw a {size} px tray icon"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_translate(margin as f32, margin as f32),
        &mut pixmap.as_mut(),
    );
    let width = (size + 2 * margin) as usize;
    let (margin, size) = (margin as usize, size as usize);
    Ok(pixmap
        .pixels()
        .chunks(width)
        .skip(margin)
        .take(size)
        .flat_map(|row| &row[margin..margin + size])
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect())
}

#[cfg(test)]
mod tests;
