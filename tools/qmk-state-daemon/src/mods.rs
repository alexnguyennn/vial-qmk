//! QMK mod bitmasks and display helpers.
//!
//! QMK mod byte layout (see `tmk_core/common/action_util.h`):
//! bit 0 = LCTL, bit 1 = LSFT, bit 2 = LALT, bit 3 = LGUI
//! bit 4 = RCTL, bit 5 = RSFT, bit 6 = RALT, bit 7 = RGUI
//!
//! For display we collapse L/R into a single letter per mod, order `CSAG`.

pub const MOD_LCTL: u8 = 1 << 0;
pub const MOD_LSFT: u8 = 1 << 1;
pub const MOD_LALT: u8 = 1 << 2;
pub const MOD_LGUI: u8 = 1 << 3;
pub const MOD_RCTL: u8 = 1 << 4;
pub const MOD_RSFT: u8 = 1 << 5;
pub const MOD_RALT: u8 = 1 << 6;
pub const MOD_RGUI: u8 = 1 << 7;

pub const MOD_CTL: u8 = MOD_LCTL | MOD_RCTL;
pub const MOD_SFT: u8 = MOD_LSFT | MOD_RSFT;
pub const MOD_ALT: u8 = MOD_LALT | MOD_RALT;
pub const MOD_GUI: u8 = MOD_LGUI | MOD_RGUI;

/// Convert a QMK mod bitmask to letters in canonical `CSAG` order.
///
/// Empty mask → empty string.
pub fn mods_to_letters(bits: u8) -> String {
    let mut s = String::with_capacity(4);
    if bits & MOD_CTL != 0 {
        s.push('C');
    }
    if bits & MOD_SFT != 0 {
        s.push('S');
    }
    if bits & MOD_ALT != 0 {
        s.push('A');
    }
    if bits & MOD_GUI != 0 {
        s.push('G');
    }
    s
}

/// Highest-precedence mod state for display coloring.
///
/// Precedence: locked > held(real) > osm > weak > none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModState {
    None,
    Weak,
    Osm,
    Held,
    Locked,
}

impl ModState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModState::None => "none",
            ModState::Weak => "weak",
            ModState::Osm => "osm",
            ModState::Held => "held",
            ModState::Locked => "locked",
        }
    }
}

pub fn resolve_state(real: u8, weak: u8, osm: u8, locked: u8) -> ModState {
    if locked != 0 {
        ModState::Locked
    } else if real != 0 {
        ModState::Held
    } else if osm != 0 {
        ModState::Osm
    } else if weak != 0 {
        ModState::Weak
    } else {
        ModState::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_empty() {
        assert_eq!(mods_to_letters(0), "");
    }

    #[test]
    fn letters_single() {
        assert_eq!(mods_to_letters(MOD_LCTL), "C");
        assert_eq!(mods_to_letters(MOD_RSFT), "S");
        assert_eq!(mods_to_letters(MOD_LALT), "A");
        assert_eq!(mods_to_letters(MOD_RGUI), "G");
    }

    #[test]
    fn letters_combined_csag_order() {
        // Ctrl + Shift + Gui in any bit order should render "CSG".
        assert_eq!(mods_to_letters(MOD_LCTL | MOD_LSFT | MOD_LGUI), "CSG");
        assert_eq!(mods_to_letters(MOD_LGUI | MOD_LSFT | MOD_LCTL), "CSG");
    }

    #[test]
    fn letters_all_four() {
        assert_eq!(
            mods_to_letters(MOD_LCTL | MOD_LSFT | MOD_LALT | MOD_LGUI),
            "CSAG"
        );
    }

    #[test]
    fn state_precedence() {
        assert_eq!(resolve_state(0, 0, 0, 0), ModState::None);
        assert_eq!(resolve_state(0, MOD_LSFT, 0, 0), ModState::Weak);
        assert_eq!(resolve_state(0, MOD_LSFT, MOD_LCTL, 0), ModState::Osm);
        assert_eq!(resolve_state(MOD_LSFT, 0, MOD_LCTL, 0), ModState::Held);
        assert_eq!(resolve_state(MOD_LSFT, 0, 0, MOD_LCTL), ModState::Locked);
    }
}
