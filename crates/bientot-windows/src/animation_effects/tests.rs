use super::*;

#[test]
fn only_animation_effects_read_as_off_reduce_motion() {
    assert!(reduces_motion(Some(false)));
    assert!(!reduces_motion(Some(true)));
    assert!(!reduces_motion(None));
}
