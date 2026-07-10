//! Layer index → short display name.
//!
//! Mirrors `enum layer` in
//! `keyboards/svalboard/keymaps/alex/keymap.c` (see line ~241).
//! Update both places together.

pub fn layer_name(index: u8) -> String {
    match index {
        0 => "BASE".to_string(),
        1 => "BASE-H".to_string(),
        2 => "FN".to_string(),
        3 => "FN-H".to_string(),
        4 => "NAS".to_string(),
        5 => "NAS-H".to_string(),
        6 => "NUM".to_string(),
        15 => "MBO".to_string(),
        n => format!("L{n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_layers() {
        assert_eq!(layer_name(0), "BASE");
        assert_eq!(layer_name(2), "FN");
        assert_eq!(layer_name(4), "NAS");
        assert_eq!(layer_name(15), "MBO");
    }

    #[test]
    fn falls_back_for_unknown() {
        assert_eq!(layer_name(9), "L9");
        assert_eq!(layer_name(12), "L12");
    }
}
