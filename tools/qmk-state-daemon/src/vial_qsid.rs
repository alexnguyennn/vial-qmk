//! Vial QMK-Settings (QSID) protocol.
//!
//! Mirrors `quantum/vial.c` command handling. Requests are 32 bytes
//! prefixed with `0xFE` + subcommand byte; responses are 32 bytes
//! starting with a status byte (0 = ok).
//!
//! See `keyboards/svalboard/keymaps/alex/tools/vial-qs.py` for the
//! reference implementation this module replaces.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};

pub const PACKET_LEN: usize = 32;
pub const VIAL_PREFIX: u8 = 0xFE;
pub const CMD_QSID_QUERY: u8 = 0x09;
pub const CMD_QSID_GET: u8 = 0x0A;
pub const CMD_QSID_SET: u8 = 0x0B;
pub const CMD_QSID_RESET: u8 = 0x0C;

/// Value width in bytes for known custom QSIDs. Extend as the keymap
/// grows. Values here must match `qmk_settings_t` field widths.
pub fn known_qsids() -> BTreeMap<u16, (&'static str, usize)> {
    let mut m = BTreeMap::new();
    m.insert(28u16, ("flow_tap_shift_delta", 2));
    m.insert(29u16, ("flow_tap_shift_min_clamp", 1));
    m
}

/// Look up the byte width for a QSID, preferring `known_qsids` and
/// falling back to a user-supplied hint.
pub fn resolve_width(qsid: u16, hint: Option<usize>) -> Result<usize> {
    if let Some(w) = hint {
        if !matches!(w, 1 | 2 | 4) {
            bail!("width must be 1, 2, or 4 bytes");
        }
        return Ok(w);
    }
    if let Some((_name, w)) = known_qsids().get(&qsid) {
        return Ok(*w);
    }
    Err(anyhow!(
        "qsid {qsid} not in known_qsids; supply an explicit width"
    ))
}

fn packet_padded(mut bytes: Vec<u8>) -> [u8; PACKET_LEN] {
    if bytes.len() > PACKET_LEN {
        panic!("payload too large: {} > {PACKET_LEN}", bytes.len());
    }
    bytes.resize(PACKET_LEN, 0);
    let mut out = [0u8; PACKET_LEN];
    out.copy_from_slice(&bytes);
    out
}

pub fn build_query_packet(cursor: u16) -> [u8; PACKET_LEN] {
    let mut v = Vec::with_capacity(4);
    v.push(VIAL_PREFIX);
    v.push(CMD_QSID_QUERY);
    v.extend_from_slice(&cursor.to_le_bytes());
    packet_padded(v)
}

pub fn build_get_packet(qsid: u16) -> [u8; PACKET_LEN] {
    let mut v = Vec::with_capacity(4);
    v.push(VIAL_PREFIX);
    v.push(CMD_QSID_GET);
    v.extend_from_slice(&qsid.to_le_bytes());
    packet_padded(v)
}

pub fn build_set_packet(qsid: u16, value: u32, width: usize) -> Result<[u8; PACKET_LEN]> {
    if !matches!(width, 1 | 2 | 4) {
        bail!("width must be 1, 2, or 4 bytes");
    }
    let max: u64 = if width == 4 {
        u32::MAX as u64
    } else {
        1u64 << (width * 8)
    };
    if (value as u64) >= max && width < 4 {
        bail!("value {value} out of range for width={width}B");
    }
    let mut v = Vec::with_capacity(4 + width);
    v.push(VIAL_PREFIX);
    v.push(CMD_QSID_SET);
    v.extend_from_slice(&qsid.to_le_bytes());
    v.extend_from_slice(&value.to_le_bytes()[..width]);
    Ok(packet_padded(v))
}

pub fn parse_get_response(buf: &[u8], width: usize) -> Result<u32> {
    if buf.is_empty() {
        bail!("empty response");
    }
    if buf[0] != 0 {
        bail!("device returned status {}", buf[0]);
    }
    if buf.len() < 1 + width {
        bail!("short response: {} < {}", buf.len(), 1 + width);
    }
    let mut arr = [0u8; 4];
    arr[..width].copy_from_slice(&buf[1..1 + width]);
    Ok(u32::from_le_bytes(arr))
}

pub fn parse_set_response(buf: &[u8]) -> Result<()> {
    if buf.is_empty() {
        bail!("empty response");
    }
    if buf[0] != 0 {
        bail!("device returned status {}", buf[0]);
    }
    Ok(())
}

/// Parse a single query-response packet (32 bytes of u16 QSIDs, `0xFFFF`
/// terminator). Returns (list_of_qsids, saw_terminator).
pub fn parse_query_response(buf: &[u8]) -> (Vec<u16>, bool) {
    let mut out = Vec::new();
    let mut end = false;
    let mut i = 0;
    while i + 2 <= buf.len() {
        let q = u16::from_le_bytes([buf[i], buf[i + 1]]);
        if q == 0xFFFF {
            end = true;
            break;
        }
        out.push(q);
        i += 2;
    }
    (out, end)
}

/// Is a given raw-HID packet a Vial QSID response (as opposed to a
/// pushed state snapshot)?
pub fn is_qsid_response(buf: &[u8]) -> bool {
    // State packets always start with 0xAB. Anything else read from the
    // device while we've sent a QSID command is treated as its
    // response. Vial doesn't correlate requests to responses, so we
    // serialize QSID RPCs at the daemon layer.
    !buf.is_empty() && buf[0] != 0xAB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_packet_shape() {
        let p = build_get_packet(28);
        assert_eq!(p.len(), PACKET_LEN);
        assert_eq!(p[0], VIAL_PREFIX);
        assert_eq!(p[1], CMD_QSID_GET);
        assert_eq!(u16::from_le_bytes([p[2], p[3]]), 28);
        assert!(p[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn set_packet_shape_u8() {
        let p = build_set_packet(29, 15, 1).unwrap();
        assert_eq!(p[0], VIAL_PREFIX);
        assert_eq!(p[1], CMD_QSID_SET);
        assert_eq!(u16::from_le_bytes([p[2], p[3]]), 29);
        assert_eq!(p[4], 15);
        assert!(p[5..].iter().all(|&b| b == 0));
    }

    #[test]
    fn set_packet_shape_u16() {
        let p = build_set_packet(28, 500, 2).unwrap();
        assert_eq!(u16::from_le_bytes([p[4], p[5]]), 500);
    }

    #[test]
    fn set_out_of_range() {
        // 1 byte max = 255
        assert!(build_set_packet(29, 300, 1).is_err());
        assert!(build_set_packet(28, 70000, 2).is_err());
    }

    #[test]
    fn query_packet_shape() {
        let p = build_query_packet(0);
        assert_eq!(p[0], VIAL_PREFIX);
        assert_eq!(p[1], CMD_QSID_QUERY);
        assert_eq!(u16::from_le_bytes([p[2], p[3]]), 0);
    }

    #[test]
    fn parse_get_ok() {
        let mut buf = vec![0u8; PACKET_LEN];
        buf[0] = 0;
        buf[1] = 0x19; // 25
        buf[2] = 0x00;
        assert_eq!(parse_get_response(&buf, 2).unwrap(), 25);
    }

    #[test]
    fn parse_get_error() {
        let mut buf = vec![0u8; PACKET_LEN];
        buf[0] = 1;
        assert!(parse_get_response(&buf, 2).is_err());
    }

    #[test]
    fn parse_query_bulk() {
        let mut buf = vec![0u8; PACKET_LEN];
        // qsids: 1, 2, 28, 29, 0xFFFF
        for (i, q) in [1u16, 2, 28, 29, 0xFFFF].iter().enumerate() {
            let bytes = q.to_le_bytes();
            buf[i * 2] = bytes[0];
            buf[i * 2 + 1] = bytes[1];
        }
        let (list, end) = parse_query_response(&buf);
        assert_eq!(list, vec![1, 2, 28, 29]);
        assert!(end);
    }

    #[test]
    fn is_qsid_response_distinguishes() {
        assert!(!is_qsid_response(&[0xAB, 1, 2]));
        assert!(is_qsid_response(&[0x00, 25]));
        assert!(!is_qsid_response(&[]));
    }

    #[test]
    fn resolve_width_hints() {
        assert_eq!(resolve_width(28, None).unwrap(), 2);
        assert_eq!(resolve_width(29, None).unwrap(), 1);
        assert_eq!(resolve_width(999, Some(4)).unwrap(), 4);
        assert!(resolve_width(999, None).is_err());
        assert!(resolve_width(999, Some(3)).is_err());
    }
}
