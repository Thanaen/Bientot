//! How big the week window opens and how its open day's card is laid out at a given width.

use gpui::{Pixels, Size, size};

use crate::theme::size;

/// The size the week opens at and the smallest it can be made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowSizes {
    pub(super) opening: Size<Pixels>,
    pub(super) minimum: Size<Pixels>,
}

impl WindowSizes {
    /// The sizes of the week on a display whose `visible` area, without its taskbar, is known:
    /// both shrink to fit it, so that a small display (1366 × 768 at 150 %) shows the window
    /// whole. On a display narrower than [`size::WEEK_WINDOW_MIN_WIDTH`], the minimum goes
    /// below it too.
    pub(super) fn fitting(visible: Option<Size<Pixels>>) -> Self {
        let opening = size(size::WEEK_WINDOW_WIDTH, size::WEEK_WINDOW_HEIGHT);
        let minimum = size(size::WEEK_WINDOW_MIN_WIDTH, size::WEEK_WINDOW_MIN_HEIGHT);
        Self {
            opening: visible.map_or(opening, |visible| opening.min(&visible)),
            minimum: visible.map_or(minimum, |visible| minimum.min(&visible)),
        }
    }
}

/// How the open day's card is laid out in a week window of a given width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Layout {
    /// The open day's content under its strip, between the day's column and its summary, the
    /// credit beside the corrections.
    Wide,
    /// The open day's content from the day's label, the credit above the corrections.
    Narrow,
}

impl Layout {
    pub(super) fn for_width(width: Pixels) -> Self {
        if width < size::WEEK_WIDE_MIN_WIDTH {
            Self::Narrow
        } else {
            Self::Wide
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::*;

    #[test]
    fn a_week_snapped_to_half_a_screen_is_narrow() {
        let table = [
            // Windows Snap: half of 1536 (1920 at 125 %), then half of 1920 at 100 %.
            (768., Layout::Narrow),
            (960., Layout::Narrow),
            (969., Layout::Narrow),
            (970., Layout::Wide),
            (1920., Layout::Wide),
        ];
        for (width, layout) in table {
            assert_eq!(Layout::for_width(px(width)), layout, "{width}");
        }
    }

    #[test]
    fn the_week_opened_at_its_width_is_wide_once_the_scale_rounds_it() {
        // At a scale of 1.2, 980 logical px are 1176 device px, read back as 979.92.
        let rounded = size::WEEK_WINDOW_WIDTH - px(0.5);
        assert_eq!(Layout::for_width(rounded), Layout::Wide);
    }

    #[test]
    fn the_week_opens_at_its_size_on_a_display_that_fits_it() {
        let expected = WindowSizes {
            opening: size(px(980.), px(760.)),
            minimum: size(px(760.), px(520.)),
        };
        assert_eq!(WindowSizes::fitting(None), expected);
        let full_hd = size(px(1920.), px(1032.));
        assert_eq!(WindowSizes::fitting(Some(full_hd)), expected);
    }

    #[test]
    fn on_a_small_display_the_week_shrinks_to_its_visible_area() {
        // 1366 × 768 at 150 %, above a taskbar of 48 physical pixels.
        let visible = size(px(910.), px(480.));
        assert_eq!(
            WindowSizes::fitting(Some(visible)),
            WindowSizes {
                opening: size(px(910.), px(480.)),
                minimum: size(px(760.), px(480.)),
            }
        );
        let tiny = size(px(640.), px(400.));
        assert_eq!(
            WindowSizes::fitting(Some(tiny)),
            WindowSizes {
                opening: tiny,
                minimum: tiny,
            }
        );
    }
}
