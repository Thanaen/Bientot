//! The offset arithmetic of the field against literal texts and widths.

use gpui::px;

use super::*;

#[test]
fn a_composition_after_a_wider_character_selects_within_itself() {
    // « é » (2 bytes, 1 unit), then « あ » (3 bytes, 1 unit) composed at byte 2, the IME's caret
    // after it.
    assert_eq!(composition_selection("éあ", 2..5, &(1..1)), 5..5);
    assert_eq!(composition_selection("éあ", 2..5, &(0..0)), 2..2);
    assert_eq!(composition_selection("éあ", 2..5, &(0..1)), 2..5);
}

#[test]
fn a_composition_of_mixed_widths_selects_its_own_characters() {
    // « 日本 » (6 bytes, 2 units), then « é😀x » (2 + 4 + 1 bytes, 1 + 2 + 1 units).
    let content = "日本é😀x";
    assert_eq!(composition_selection(content, 6..13, &(1..3)), 8..12);
    assert_eq!(composition_selection(content, 6..13, &(4..4)), 13..13);
}

#[test]
fn a_composition_selection_past_its_end_stops_at_its_end() {
    // The field kept only « あ » of « あい »: the IME's caret after « い » lands after « あ ».
    assert_eq!(composition_selection("aあ", 1..4, &(2..2)), 4..4);
}

#[test]
fn a_composition_before_the_rest_selects_within_itself() {
    // « あ » composed at the start of « é ».
    assert_eq!(composition_selection("あé", 0..3, &(1..1)), 3..3);
}

#[test]
fn a_line_narrower_than_the_field_does_not_scroll() {
    assert_eq!(scroll_for(px(80.), px(0.), px(100.), px(80.)), px(0.));
    assert_eq!(scroll_for(px(0.), px(0.), px(100.), px(80.)), px(0.));
    // A scroll left from a longer text goes back.
    assert_eq!(scroll_for(px(40.), px(30.), px(100.), px(80.)), px(0.));
}

#[test]
fn a_caret_past_the_right_edge_scrolls_it_into_view() {
    // The caret, 1 px wide, ends at the right edge.
    assert_eq!(scroll_for(px(250.), px(0.), px(100.), px(250.)), px(151.));
    assert_eq!(scroll_for(px(120.), px(0.), px(100.), px(250.)), px(21.));
}

#[test]
fn a_caret_past_the_left_edge_scrolls_back_to_it() {
    // Home.
    assert_eq!(scroll_for(px(0.), px(151.), px(100.), px(250.)), px(0.));
    assert_eq!(scroll_for(px(60.), px(151.), px(100.), px(250.)), px(60.));
}

#[test]
fn a_caret_in_view_keeps_the_scroll() {
    assert_eq!(scroll_for(px(200.), px(151.), px(100.), px(250.)), px(151.));
    assert_eq!(scroll_for(px(151.), px(151.), px(100.), px(250.)), px(151.));
}

#[test]
fn a_shorter_line_does_not_leave_room_after_its_end() {
    // Deleting the end of the line: its end comes back to the right edge.
    assert_eq!(scroll_for(px(180.), px(151.), px(100.), px(180.)), px(81.));
}
