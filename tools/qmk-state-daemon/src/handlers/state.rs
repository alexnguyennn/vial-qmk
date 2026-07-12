//! Handler for `0xAB` state snapshot packets.
//!
//! Packet layout (v1):
//! ```text
//! 0: msg_id = 0xAB
//! 1: version = 1
//! 2: reason bitfield (0x01 layer, 0x02 default, 0x04 mods, 0x08 osm, 0x10 initial/query)
//! 3: top_layer
//! 4: default_layer
//! 5..9: layer_state (u32 LE)
//! 9:  real_mods
//! 10: weak_mods
//! 11: oneshot_mods
//! 12: locked_mods
//! 13: caps_word_active (0|1)
//! 14..32: reserved
//! ```

use anyhow::{ensure, Result};
use serde::Serialize;

use crate::layer_names::layer_name;
use crate::mods::{mods_to_letters, resolve_state};
use crate::packet::PacketHandler;

pub const STATE_MSG_ID: u8 = 0xAB;
pub const STATE_VERSION: u8 = 1;
pub const STATE_PACKET_LEN: usize = 32;

pub const REASON_LAYER: u8 = 0x01;
pub const REASON_DEFAULT_LAYER: u8 = 0x02;
pub const REASON_MODS: u8 = 0x04;
pub const REASON_OSM: u8 = 0x08;
pub const REASON_INITIAL: u8 = 0x10;
pub const REASON_CAPS_WORD: u8 = 0x20;

#[derive(Debug, Serialize, PartialEq)]
pub struct StatePayload {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub version: u8,
    pub reason: u8,
    pub reason_flags: Vec<&'static str>,
    pub top_layer: u8,
    pub top_layer_name: String,
    pub default_layer: u8,
    pub default_layer_name: String,
    pub layer_state: u32,
    pub real_mods: u8,
    pub weak_mods: u8,
    pub oneshot_mods: u8,
    pub locked_mods: u8,
    pub mods_letters: String,
    pub mods_state: &'static str,
    pub caps_word_active: bool,
    pub caps_word_state: &'static str,
}

pub struct StateHandler;

impl StateHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StateHandler {
    fn default() -> Self {
        Self::new()
    }
}

fn reason_flags(reason: u8) -> Vec<&'static str> {
    let mut out = Vec::new();
    if reason & REASON_LAYER != 0 {
        out.push("layer");
    }
    if reason & REASON_DEFAULT_LAYER != 0 {
        out.push("default_layer");
    }
    if reason & REASON_MODS != 0 {
        out.push("mods");
    }
    if reason & REASON_OSM != 0 {
        out.push("oneshot");
    }
    if reason & REASON_INITIAL != 0 {
        out.push("initial");
    }
    if reason & REASON_CAPS_WORD != 0 {
        out.push("caps_word");
    }
    out
}

impl PacketHandler for StateHandler {
    fn msg_id(&self) -> u8 {
        STATE_MSG_ID
    }

    fn decode(&self, buf: &[u8]) -> Result<serde_json::Value> {
        ensure!(
            buf.len() >= STATE_PACKET_LEN,
            "state packet too short: {} < {}",
            buf.len(),
            STATE_PACKET_LEN
        );
        ensure!(buf[0] == STATE_MSG_ID, "wrong msg_id 0x{:02X}", buf[0]);
        ensure!(
            buf[1] == STATE_VERSION,
            "unsupported state packet version {}",
            buf[1]
        );

        let reason = buf[2];
        let top_layer = buf[3];
        let default_layer = buf[4];
        let layer_state = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let real_mods = buf[9];
        let weak_mods = buf[10];
        let oneshot_mods = buf[11];
        let locked_mods = buf[12];
        let caps_word_active = buf[13] != 0;

        let display_bits = real_mods | weak_mods | oneshot_mods | locked_mods;
        let mods_letters = mods_to_letters(display_bits);
        let mods_state = resolve_state(real_mods, weak_mods, oneshot_mods, locked_mods).as_str();

        let payload = StatePayload {
            kind: "state",
            version: buf[1],
            reason,
            reason_flags: reason_flags(reason),
            top_layer,
            top_layer_name: layer_name(top_layer),
            default_layer,
            default_layer_name: layer_name(default_layer),
            layer_state,
            real_mods,
            weak_mods,
            oneshot_mods,
            locked_mods,
            mods_letters,
            mods_state,
            caps_word_active,
            caps_word_state: if caps_word_active {
                "active"
            } else {
                "inactive"
            },
        };
        Ok(serde_json::to_value(payload)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::{MOD_LCTL, MOD_LSFT};

    fn fixture_packet() -> [u8; STATE_PACKET_LEN] {
        let mut buf = [0u8; STATE_PACKET_LEN];
        buf[0] = STATE_MSG_ID;
        buf[1] = STATE_VERSION;
        buf[2] = REASON_LAYER | REASON_MODS;
        buf[3] = 4; // NAS
        buf[4] = 0; // BASE
                    // layer_state = (1<<0) | (1<<4) = 0x11
        buf[5..9].copy_from_slice(&0x0000_0011u32.to_le_bytes());
        buf[9] = MOD_LCTL | MOD_LSFT; // held CS
        buf[10] = 0;
        buf[11] = 0;
        buf[12] = 0;
        buf
    }

    #[test]
    fn decodes_state_from_fixture() {
        let buf = fixture_packet();
        let v = StateHandler::new().decode(&buf).unwrap();
        assert_eq!(v["type"], "state");
        assert_eq!(v["version"], 1);
        assert_eq!(v["top_layer"], 4);
        assert_eq!(v["top_layer_name"], "NAS");
        assert_eq!(v["default_layer_name"], "BASE");
        assert_eq!(v["layer_state"], 0x11);
        assert_eq!(v["mods_letters"], "CS");
        assert_eq!(v["mods_state"], "held");
        assert_eq!(v["caps_word_active"], false);
        assert_eq!(v["caps_word_state"], "inactive");
        let flags: Vec<String> = serde_json::from_value(v["reason_flags"].clone()).unwrap();
        assert_eq!(flags, vec!["layer", "mods"]);
    }

    #[test]
    fn decodes_caps_word_fields() {
        let mut buf = fixture_packet();
        buf[2] = REASON_CAPS_WORD;
        buf[13] = 1;
        let v = StateHandler::new().decode(&buf).unwrap();
        assert_eq!(v["caps_word_active"], true);
        assert_eq!(v["caps_word_state"], "active");
        let flags: Vec<String> = serde_json::from_value(v["reason_flags"].clone()).unwrap();
        assert_eq!(flags, vec!["caps_word"]);
    }

    #[test]
    fn rejects_short_buffer() {
        let buf = [0xAB, 0x01, 0, 0];
        assert!(StateHandler::new().decode(&buf).is_err());
    }

    #[test]
    fn rejects_wrong_msg_id() {
        let mut buf = fixture_packet();
        buf[0] = 0xCC;
        assert!(StateHandler::new().decode(&buf).is_err());
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut buf = fixture_packet();
        buf[1] = 99;
        assert!(StateHandler::new().decode(&buf).is_err());
    }

    #[test]
    fn no_mods_yields_empty_letters_and_none_state() {
        let mut buf = fixture_packet();
        buf[9] = 0;
        buf[10] = 0;
        let v = StateHandler::new().decode(&buf).unwrap();
        assert_eq!(v["mods_letters"], "");
        assert_eq!(v["mods_state"], "none");
    }

    #[test]
    fn locked_beats_held() {
        let mut buf = fixture_packet();
        buf[9] = MOD_LSFT; // held
        buf[12] = MOD_LCTL; // locked
        let v = StateHandler::new().decode(&buf).unwrap();
        assert_eq!(v["mods_state"], "locked");
        assert_eq!(v["mods_letters"], "CS");
    }
}
