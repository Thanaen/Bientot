use super::*;

const SIZES: [u32; 7] = [16, 20, 24, 28, 32, 40, 48];
const COLOURS: [TrayColour; 4] = [
    TrayColour::Green,
    TrayColour::Orange,
    TrayColour::Red,
    TrayColour::Blue,
];

fn pixel(image: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * size + x) * 4) as usize;
    image[at..at + 4].try_into().unwrap()
}

fn hex(colour: TrayColour) -> [u8; 4] {
    let value = u32::from_str_radix(&fill(colour)[1..], 16).unwrap();
    let [_, r, g, b] = value.to_be_bytes();
    [r, g, b, 255]
}

#[test]
fn every_icon_is_drawn_at_the_size_asked_for() {
    for colour in COLOURS {
        for degraded in [false, true] {
            for taskbar in [Taskbar::Dark, Taskbar::Light] {
                for size in SIZES {
                    let image = rgba(TrayIcon { colour, degraded }, size, taskbar).unwrap();
                    assert_eq!(image.len(), (size * size * 4) as usize, "{colour:?} {size}");
                }
            }
        }
    }
}

#[test]
fn the_disc_fills_the_icon_in_the_day_colour_with_a_white_symbol() {
    for colour in COLOURS {
        for size in SIZES {
            let icon = TrayIcon {
                colour,
                degraded: false,
            };
            let image = rgba(icon, size, Taskbar::Dark).unwrap();
            assert_eq!(pixel(&image, size, 0, 0)[3], 0, "{colour:?} {size}: corner");
            // Just inside the disc's left edge, away from the symbol.
            assert_eq!(
                pixel(&image, size, 2, size / 2),
                hex(colour),
                "{colour:?} {size}: disc"
            );
            assert!(
                image.chunks(4).any(|pixel| pixel == [255, 255, 255, 255]),
                "{colour:?} {size}: no white symbol"
            );
        }
    }
}

#[test]
fn the_orange_mark_is_drawn_on_whole_pixels() {
    for size in [16, 20, 24, 32] {
        let icon = TrayIcon {
            colour: TrayColour::Orange,
            degraded: false,
        };
        let image = rgba(icon, size, Taskbar::Dark).unwrap();
        let column = (0..size).map(|y| pixel(&image, size, size / 2, y));
        let white = column
            .filter(|pixel| *pixel == [255, 255, 255, 255])
            .count();
        // The bar and its dot, at full white rather than smeared over two half-lit columns.
        assert!(
            white as f32 >= size as f32 * 0.5,
            "{size}: {white} white pixels"
        );
    }
}

#[test]
fn a_source_in_trouble_adds_a_badge_that_contrasts_with_the_taskbar() {
    for colour in COLOURS {
        for size in SIZES {
            let corner = size - 1 - size / 8;
            let plain = rgba(
                TrayIcon {
                    colour,
                    degraded: false,
                },
                size,
                Taskbar::Dark,
            )
            .unwrap();
            assert_ne!(pixel(&plain, size, corner, corner), [255, 255, 255, 255]);
            for (taskbar, text) in [
                (Taskbar::Dark, [255, 255, 255, 255]),
                (Taskbar::Light, [0x1A, 0x1A, 0x1A, 255]),
            ] {
                let icon = TrayIcon {
                    colour,
                    degraded: true,
                };
                let image = rgba(icon, size, taskbar).unwrap();
                assert_eq!(
                    pixel(&image, size, corner, corner),
                    text,
                    "{colour:?} {size} {taskbar:?}"
                );
            }
        }
    }
}

#[test]
fn the_badge_leaves_the_symbol_centred() {
    for colour in COLOURS {
        for size in SIZES {
            let plain = rgba(
                TrayIcon {
                    colour,
                    degraded: false,
                },
                size,
                Taskbar::Dark,
            )
            .unwrap();
            let badged = rgba(
                TrayIcon {
                    colour,
                    degraded: true,
                },
                size,
                Taskbar::Dark,
            )
            .unwrap();
            // Away from the bottom-right corner, the two icons are the same pixel for pixel. The
            // disc's smoothed rim is left out: it shifts slightly when it goes through a mask.
            let clear = size / 2 + size / 8;
            let half = size as f32 / 2.;
            for y in 0..size {
                for x in 0..size {
                    let rim = (x as f32 + 0.5 - half).hypot(y as f32 + 0.5 - half) > half - 2.;
                    if (x < clear || y < clear) && !rim {
                        assert_eq!(
                            pixel(&badged, size, x, y),
                            pixel(&plain, size, x, y),
                            "{colour:?} {size} at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_badge_keeps_clear_of_the_symbol() {
    for colour in COLOURS {
        for size in SIZES {
            let icon = TrayIcon {
                colour,
                degraded: true,
            };
            // On a light taskbar the badge is dark, so any white left is the symbol's.
            let image = rgba(icon, size, Taskbar::Light).unwrap();
            let plain = rgba(
                TrayIcon {
                    colour,
                    degraded: false,
                },
                size,
                Taskbar::Light,
            )
            .unwrap();
            let white = |image: &[u8]| {
                image
                    .chunks(4)
                    .filter(|pixel| {
                        pixel[0] > 128 && pixel[1] > 128 && pixel[2] > 128 && pixel[3] == 255
                    })
                    .count()
            };
            assert_eq!(white(&image), white(&plain), "{colour:?} {size}");
        }
    }
}

/// FNV-1a, 64 bits: a digest that stays the same from one Rust release to the next.
fn digest(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn the_icons_for_a_dark_or_light_taskbar_keep_their_pixels() {
    let mut bytes = Vec::new();
    for taskbar in [Taskbar::Dark, Taskbar::Light] {
        for colour in COLOURS {
            for degraded in [false, true] {
                for size in [16, 20, 22, 24, 28, 32, 40, 48] {
                    bytes.extend(rgba(TrayIcon { colour, degraded }, size, taskbar).unwrap());
                }
            }
        }
    }
    assert_eq!(format!("{:016x}", digest(bytes)), "bdca388cad8eb5dd");
}

/// The sizes a Linux panel asks for (`bientot_linux::tray::ICON_SIZES`), and the badge's ring
/// radius at each, which its centre sits that far from the bottom and right edges.
const PANEL_SIZES: [(u32, u32); 5] = [(16, 3), (22, 4), (24, 4), (32, 6), (48, 8)];
const INK_PIXEL: [u8; 4] = [0x1A, 0x1A, 0x1A, 255];

fn unknown(colour: TrayColour, degraded: bool, size: u32) -> Vec<u8> {
    rgba(TrayIcon { colour, degraded }, size, Taskbar::Unknown).unwrap()
}

/// How much of the pixel is white, from 0 to 1: a blend with the disc or the dot counts less.
fn whiteness(pixel: [u8; 4]) -> f32 {
    let lightest = pixel[..3].iter().min().copied().unwrap_or(0);
    f32::from(pixel[3]) / 255. * f32::from(lightest) / 255.
}

/// The first pixel that is not the dot's dark colour or a blend into it.
fn first_light(mut pixels: impl Iterator<Item = [u8; 4]>) -> Option<[u8; 4]> {
    pixels.find(|pixel| pixel[..3].iter().any(|channel| *channel > 0x40))
}

fn is_white(pixel: [u8; 4]) -> bool {
    pixel[3] == 255 && pixel[..3].iter().all(|channel| *channel >= 0xF0)
}

#[test]
fn every_icon_for_an_unknown_panel_is_drawn_at_the_size_asked_for() {
    for colour in COLOURS {
        for degraded in [false, true] {
            for (size, _) in PANEL_SIZES {
                assert_eq!(
                    unknown(colour, degraded, size).len(),
                    (size * size * 4) as usize,
                    "{colour:?} {size}"
                );
            }
        }
    }
}

#[test]
fn on_an_unknown_panel_a_white_rim_rings_the_disc_at_every_angle() {
    for colour in COLOURS {
        for degraded in [false, true] {
            for (size, _) in PANEL_SIZES {
                let image = unknown(colour, degraded, size);
                let half = size as f32 / 2.;
                for degree in 0..360 {
                    // The badge takes the bottom-right corner, y down.
                    if degraded && (10..=80).contains(&degree) {
                        continue;
                    }
                    let (sin, cos) = (degree as f32).to_radians().sin_cos();
                    // The whitest pixel on the ray through the rim.
                    let white = (0..=60)
                        .map(|step| half - 3. + step as f32 * 0.05)
                        .filter_map(|radius| {
                            let x = (half + radius * cos).floor();
                            let y = (half + radius * sin).floor();
                            let inside =
                                (0. ..size as f32).contains(&x) && (0. ..size as f32).contains(&y);
                            inside.then(|| whiteness(pixel(&image, size, x as u32, y as u32)))
                        })
                        .fold(0., f32::max);
                    assert!(
                        white >= 0.8,
                        "{colour:?} {size} degraded {degraded} at {degree}°: {white}"
                    );
                }
            }
        }
    }
}

#[test]
fn on_an_unknown_panel_the_symbol_is_the_one_a_taskbar_shows() {
    for colour in COLOURS {
        for (size, _) in PANEL_SIZES {
            let image = unknown(colour, false, size);
            let today = rgba(
                TrayIcon {
                    colour,
                    degraded: false,
                },
                size,
                Taskbar::Dark,
            )
            .unwrap();
            // Every pixel wholly inside the rim, symbol and disc.
            let half = size as f32 / 2.;
            for y in 0..size {
                for x in 0..size {
                    let far = |v: u32| (v as f32 - half).abs().max((v as f32 + 1. - half).abs());
                    if far(x).hypot(far(y)) <= half - 2. {
                        assert_eq!(
                            pixel(&image, size, x, y),
                            pixel(&today, size, x, y),
                            "{colour:?} {size} at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn on_an_unknown_panel_the_badge_is_a_dark_dot_in_a_white_ring_inside_the_icon() {
    for colour in COLOURS {
        for (size, ring) in PANEL_SIZES {
            let image = unknown(colour, true, size);
            let centre = size - ring;
            let edge = size - 1;
            let at = |x, y| pixel(&image, size, x, y);
            let context = format!("{colour:?} {size}");
            assert_eq!(at(centre, centre), INK_PIXEL, "{context}: dot");
            assert_eq!(at(centre - 1, centre - 1), INK_PIXEL, "{context}: dot");
            // Towards the disc, the ring parts it from the dot: white before any other colour.
            let left = first_light((0..centre).rev().map(|x| at(x, centre)));
            let up = first_light((0..centre).rev().map(|y| at(centre, y)));
            assert!(
                left.is_some_and(is_white),
                "{context}: left of the dot {left:?}"
            );
            assert!(up.is_some_and(is_white), "{context}: above the dot {up:?}");
            // Towards the panel, the ring touches the right and bottom edges, over at most two
            // opaque pixels each, and curves away before the corner.
            for (side, line) in [
                ("right", (0..size).map(|y| at(edge, y)).collect::<Vec<_>>()),
                ("bottom", (0..size).map(|x| at(x, edge)).collect()),
            ] {
                let touch = line[centre as usize];
                assert!(
                    touch[3] >= 200 && touch[..3].iter().all(|channel| *channel >= 0xF0),
                    "{context}: the ring does not reach the {side} edge: {touch:?}"
                );
                let opaque = line[(size / 2 + size / 8) as usize..]
                    .iter()
                    .filter(|pixel| pixel[3] == 255)
                    .count();
                assert!(opaque <= 2, "{context}: {opaque} opaque on the {side} edge");
            }
            assert_eq!(at(edge, edge)[3], 0, "{context}: corner");
        }
    }
}

#[test]
fn on_an_unknown_panel_the_disc_is_as_round_on_each_side() {
    for (size, _) in PANEL_SIZES {
        let image = unknown(TrayColour::Green, false, size);
        let alpha = |x, y| i16::from(pixel(&image, size, x, y)[3]);
        for y in 0..size {
            for x in 0..size {
                let mirrored = [alpha(size - 1 - x, y), alpha(x, size - 1 - y)];
                // One step of the rasteriser's 16 levels of coverage.
                assert!(
                    mirrored
                        .iter()
                        .all(|other| (alpha(x, y) - other).abs() <= 16),
                    "{size} at ({x}, {y}): {} against {mirrored:?}",
                    alpha(x, y)
                );
            }
        }
    }
}

/// The radius inside which a pixel's centre belongs to the disc, at `size`.
fn disc_radius(size: u32) -> f32 {
    size as f32 / 2. - if size >= 32 { 2. } else { 1.5 }
}

fn from_centre(size: u32, x: u32, y: u32) -> f32 {
    let half = size as f32 / 2.;
    (x as f32 + 0.5 - half).hypot(y as f32 + 0.5 - half)
}

#[test]
fn on_an_unknown_panel_no_sliver_of_the_disc_is_left_between_the_badge_and_the_rim() {
    for colour in COLOURS {
        for (size, _) in PANEL_SIZES {
            let plain = unknown(colour, false, size);
            let badged = unknown(colour, true, size);
            let disc = |x, y| pixel(&badged, size, x, y) == hex(colour);
            let ring =
                |x, y| is_white(pixel(&badged, size, x, y)) && !is_white(pixel(&plain, size, x, y));
            let rim = |x, y| {
                is_white(pixel(&badged, size, x, y)) && from_centre(size, x, y) >= disc_radius(size)
            };
            let context = format!("{colour:?} {size}");
            for across in [true, false] {
                for line in 0..size {
                    let at = |step: u32| if across { (step, line) } else { (line, step) };
                    let mut step = 0;
                    while step < size {
                        let start = step;
                        while step < size && disc(at(step).0, at(step).1) {
                            step += 1;
                        }
                        if step == start {
                            step += 1;
                            continue;
                        }
                        if start == 0 || step == size || step - start > 2 {
                            continue;
                        }
                        let (before, after) = (at(start - 1), at(step));
                        let pinched = (ring(before.0, before.1) && rim(after.0, after.1))
                            || (rim(before.0, before.1) && ring(after.0, after.1));
                        assert!(
                            !pinched,
                            "{context}: {} disc pixels from {:?} between the ring and the rim",
                            step - start,
                            at(start)
                        );
                    }
                }
            }
            // Where the symbol does not reach, the corner is the same either side of its diagonal.
            for y in 0..size {
                for x in 0..size {
                    let bare = |x, y| pixel(&plain, size, x, y) == hex(colour);
                    if bare(x, y) && bare(y, x) {
                        assert_eq!(disc(x, y), disc(y, x), "{context} at ({x}, {y})");
                    }
                }
            }
        }
    }
}

#[test]
fn on_an_unknown_panel_a_whole_pixel_of_the_disc_parts_the_symbol_from_the_badge() {
    for colour in COLOURS {
        for (size, ring) in PANEL_SIZES {
            let plain = unknown(colour, false, size);
            let badged = unknown(colour, true, size);
            let inside = |x, y| from_centre(size, x, y) < disc_radius(size);
            let before = |x, y| pixel(&plain, size, x, y);
            let after = |x, y| pixel(&badged, size, x, y);
            // A pixel the symbol touches, even in part, that the badge left as it was.
            let symbol =
                |x, y| inside(x, y) && before(x, y) != hex(colour) && after(x, y) == before(x, y);
            // A pixel the badge drew, ring or dot.
            let badge =
                |x, y| inside(x, y) && after(x, y) != before(x, y) && after(x, y) != hex(colour);
            let centre = (size - ring) as f32;
            for y in 0..size {
                for x in 0..size {
                    let context = format!("{colour:?} {size} at ({x}, {y})");
                    if after(x, y) != before(x, y) {
                        let reach = (x as f32 + 0.5 - centre).hypot(y as f32 + 0.5 - centre);
                        assert!(
                            reach <= ring as f32 + 2.5,
                            "{context}: changed {reach} px from the badge"
                        );
                    }
                    if !symbol(x, y) {
                        continue;
                    }
                    for (dx, dy) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        let (nx, ny) = (x.checked_add_signed(dx), y.checked_add_signed(dy));
                        if let (Some(nx), Some(ny)) = (nx, ny)
                            && nx < size
                            && ny < size
                        {
                            assert!(
                                !badge(nx, ny),
                                "{context}: touches the badge at ({nx}, {ny})"
                            );
                        }
                    }
                }
            }
        }
    }
}
