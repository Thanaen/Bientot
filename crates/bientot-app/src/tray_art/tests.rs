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
