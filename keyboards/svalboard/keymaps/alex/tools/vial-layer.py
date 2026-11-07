#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "hidapi>=0.14",
# ]
# ///
"""vial-layer: read the LIVE dynamic keymap straight off the keyboard.

Ground truth for "what is actually bound right now", as opposed to keymap.c
(the compiled fallback) or a stale .vil.

Two fork-specific quirks this tool exists to work around:
  * quantum/via.c routes VIA commands with data[0] = command id (no VIA
    header byte) and writes the keycode reply at command_data[3..4], i.e.
    bytes 4..5 — stock VIA uses command_data[0..1].
  * The board pushes 0xAB qmk-state broadcast packets asynchronously, which
    must be skipped so they are not mistaken for a reply.

Stop the qmk-state-daemon first (it owns raw-HID exclusively).

Usage:
  uv run --project . vial-layer.py info              # protocol/uid/rows
  uv run --project . vial-layer.py dump --layer 0    # physical layout
  uv run --project . vial-layer.py taps --layer 0    # only tap-hold keys
  uv run --project . vial-layer.py cell --row 0 --col 2
"""
from __future__ import annotations

import argparse
import json
import struct
import subprocess
import sys
from pathlib import Path
from typing import Iterable

try:
    import hid  # type: ignore
except ModuleNotFoundError:  # allow offline use (diff / re-decode a saved fingerprint)
    hid = None

VIAL_SERIAL_MAGIC = "vial:"
RAW_USAGE_PAGE = 0xFF60
RAW_USAGE = 0x61
MSG_LEN = 32

CMD_VIAL_PREFIX = 0xFE
VIAL_GET_KEYBOARD_ID = 0x00
VIAL_DYNAMIC_ENTRY_OP = 0x0D
ENTRY_GET_COUNT = 0x00
ENTRY_COMBO_GET = 0x03
VIA_ID_GET_KEYCODE = 0x04
SVAL_PREFIX = 0xEE
SVAL_GET_FW_VERSION = 0x02

MATRIX_ROWS = 10
MATRIX_COLS = 6
INFO_JSON = Path(__file__).resolve().parents[3] / "info.json"

# quantum/keycodes.h enum qk_keycode_ranges (this tree's layout)
R_MODS_LO, R_MODS_HI = 0x0100, 0x1FFF
R_MOD_TAP_LO, R_MOD_TAP_HI = 0x2000, 0x3FFF
R_LAYER_TAP_LO, R_LAYER_TAP_HI = 0x4000, 0x4FFF
R_LAYER_MOD_LO, R_LAYER_MOD_HI = 0x5000, 0x51FF

MOD_NAMES = {1: "C", 2: "S", 4: "A", 8: "G", 16: "c", 32: "s", 64: "a", 128: "g"}

# Basic keycodes 0x00-0xFF, checked against quantum/keycodes.h in this tree.
# NOTE 0x0001 is KC_TRANSPARENT (not KC_ERRONE), and the 0x4C+ block is nav/keypad:
# an earlier revision of this table had it shifted by one and called 0xE0-0xE7 "MOUSE".
BASIC = {
    0x00: "____", 0x01: "TRNS",
    0x04: "A", 0x05: "B", 0x06: "C", 0x07: "D", 0x08: "E",
    0x09: "F", 0x0A: "G", 0x0B: "H", 0x0C: "I", 0x0D: "J", 0x0E: "K", 0x0F: "L",
    0x10: "M", 0x11: "N", 0x12: "O", 0x13: "P", 0x14: "Q", 0x15: "R", 0x16: "S",
    0x17: "T", 0x18: "U", 0x19: "V", 0x1A: "W", 0x1B: "X", 0x1C: "Y", 0x1D: "Z",
    0x1E: "1", 0x1F: "2", 0x20: "3", 0x21: "4", 0x22: "5", 0x23: "6", 0x24: "7",
    0x25: "8", 0x26: "9", 0x27: "0", 0x28: "ENTER", 0x29: "ESC", 0x2A: "BSPC",
    0x2B: "TAB", 0x2C: "SPC", 0x2D: "-", 0x2E: "=", 0x2F: "[", 0x30: "]",
    0x31: "\\", 0x32: "NUHS", 0x33: ";", 0x34: "'", 0x35: "`", 0x36: ",", 0x37: ".",
    0x38: "/", 0x39: "CAPS", 0x3A: "F1", 0x3B: "F2", 0x3C: "F3", 0x3D: "F4",
    0x3E: "F5", 0x3F: "F6", 0x40: "F7", 0x41: "F8", 0x42: "F9", 0x43: "F10",
    0x44: "F11", 0x45: "F12", 0x46: "PSCR", 0x47: "SCRL", 0x48: "PAUSE",
    0x49: "INS", 0x4A: "HOME", 0x4B: "PGUP", 0x4C: "DEL", 0x4D: "END", 0x4E: "PGDN",
    0x4F: "RIGHT", 0x50: "LEFT", 0x51: "DOWN", 0x52: "UP", 0x53: "NUMLOCK",
    0x54: "KP/", 0x55: "KP*", 0x56: "KP-", 0x57: "KP+", 0x58: "KPENT",
    0x59: "KP1", 0x5A: "KP2", 0x5B: "KP3", 0x5C: "KP4", 0x5D: "KP5", 0x5E: "KP6",
    0x5F: "KP7", 0x60: "KP8", 0x61: "KP9", 0x62: "KP0", 0x63: "KP.",
    0x64: "NUBS", 0x65: "APP", 0x66: "POWER", 0x67: "KP=",
    0x68: "F13", 0x69: "F14", 0x6A: "F15", 0x6B: "F16", 0x6C: "F17", 0x6D: "F18",
    0x6E: "F19", 0x6F: "F20", 0x70: "F21", 0x71: "F22", 0x72: "F23", 0x73: "F24", 0x76: "MENU",
    0xA5: "POWER", 0xA6: "SLEEP", 0xA7: "WAKE",
    0xA8: "MUTE", 0xA9: "VOL+", 0xAA: "VOL-", 0xAB: "NEXT", 0xAC: "PREV", 0xAD: "STOP",
    0xAE: "PLAY", 0xAF: "SELECT", 0xB0: "EJECT", 0xB1: "MAIL", 0xB2: "CALC", 0xB3: "PC",
    0xB4: "WWW+", 0xB5: "WWW", 0xB6: "WWW<", 0xB7: "WWW>", 0xB8: "WWWX", 0xB9: "WWW#",
    0xBA: "WWW*", 0xBB: "FFWD", 0xBC: "REWD", 0xBD: "BRT+", 0xBE: "BRT-", 0xBF: "CTRLPANEL",
    0xC0: "ASSIST", 0xC1: "MISSION", 0xC2: "LAUNCH",
    0xD1: "BTN1", 0xD2: "BTN2", 0xD3: "BTN3", 0xD4: "BTN4", 0xD5: "BTN5",
    0xE0: "LCTL", 0xE1: "LSFT", 0xE2: "LALT", 0xE3: "LGUI",
    0xE4: "RCTL", 0xE5: "RSFT", 0xE6: "RALT", 0xE7: "RGUI",
}


def mod_str(mods: int) -> str:
    return "".join(MOD_NAMES[b] for b in sorted(MOD_NAMES) if mods & b)


def decode(kc: int) -> str:
    if kc in BASIC:
        return BASIC[kc]
    if kc == 0x0000:
        return "NO"
    if R_MODS_LO <= kc <= R_MODS_HI:
        base = BASIC.get(kc & 0xFF, hex(kc & 0xFF))
        mods = mod_str((kc & 0x1F00) >> 8)
        return f"{mods}({base})" if (kc & 0xFF) else f"{mods}"
    if R_MOD_TAP_LO <= kc <= R_MOD_TAP_HI:
        return f"MT({mod_str((kc & 0x1F00) >> 8)}|{BASIC.get(kc & 0xFF, hex(kc & 0xFF))})"
    if R_LAYER_TAP_LO <= kc <= R_LAYER_TAP_HI:
        return f"LT({(kc & 0x0F00) >> 8},{BASIC.get(kc & 0xFF, hex(kc & 0xFF))})"
    if R_LAYER_MOD_LO <= kc <= R_LAYER_MOD_HI:
        return f"LM({(kc & 0x0100) >> 8},{mod_str((kc & 0x1F00) >> 8)})"
    if 0x5200 <= kc <= 0x521F:
        return f"TO({kc & 0x1F})"
    if 0x5220 <= kc <= 0x523F:
        return f"MO({kc & 0x1F})"
    if 0x5240 <= kc <= 0x525F:
        return f"DF({kc & 0x1F})"
    if 0x5260 <= kc <= 0x527F:
        return f"TG({kc & 0x1F})"
    if 0x5280 <= kc <= 0x529F:
        return f"OSL({kc & 0x1F})"
    if 0x52A0 <= kc <= 0x52BF:
        return f"OSM({mod_str((kc & 0x1F0) >> 4)})"
    if 0x52C0 <= kc <= 0x52DF:
        return f"TT({kc & 0x1F})"
    if 0x5700 <= kc <= 0x57FF:
        return f"TD({kc & 0xFF})"
    if 0x7700 <= kc <= 0x777F:
        return f"M({kc & 0x7F})"
    if 0x7C00 <= kc <= 0x7DFF:
        return {0x7C18: "SC_LSPO", 0x7C19: "SC_RSPC", 0x7C1A: "SC_LAPO", 0x7C1B: "SC_RAPC",
                0x7C16: "GRAVE_ESC", 0x7C17: "VELOCITOG"}.get(kc, f"MAGIC_0x{kc - 0x7C00:02X}")
    if kc == 0x7E00:
        return "SV_QMK_STATE_TOGGLE"
    if 0x7E00 <= kc <= 0x7E3F:
        return f"KB_{kc - 0x7E00}"
    if kc == 0xFFFF:
        return "????"
    return f"0x{kc:04X}"


def labels() -> dict[tuple[int, int], str]:
    layout = json.loads(INFO_JSON.read_text())["layouts"]["LAYOUT"]["layout"]
    return {(k["matrix"][0], k["matrix"][1]): k["label"] for k in layout}


def find_vial_path(vid: int | None, pid: int | None, serial: str | None = None, index: int | None = None) -> tuple[str, str]:
    matches = []
    for info in hid.enumerate(vid or 0, pid or 0):
        if info.get("usage_page") != RAW_USAGE_PAGE or info.get("usage") != RAW_USAGE:
            continue
        if VIAL_SERIAL_MAGIC in (info.get("serial_number") or ""):
            matches.append(info)
    if not matches:
        raise SystemExit("no vial keyboard found on raw-hid interface")
    if serial:
        matches = [m for m in matches if serial in (m.get("serial_number") or "")]
        if not matches:
            raise SystemExit(f"no vial device with serial containing {serial!r}")
    if index is not None:
        chosen = matches[index]
    else:
        if len(matches) > 1:
            print("multiple vial devices; pass --serial or --index. Using the first:", file=sys.stderr)
            for m in matches:
                print(f"  {m.get('serial_number')}  {m.get('product_string')}", file=sys.stderr)
        chosen = matches[0]
    return chosen["path"], chosen.get("serial_number", "?")


def send(dev: hid.device, payload: bytes) -> bytes:
    payload = payload + b"\x00" * (MSG_LEN - len(payload))
    dev.write(b"\x00" + payload)
    for _ in range(12):
        resp = dev.read(MSG_LEN, timeout_ms=500)
        if not resp:
            raise SystemExit("no response from device (timeout)")
        if resp[0] != 0xAB:  # skip qmk-state broadcast packets
            return bytes(resp)
    raise SystemExit("only state packets received")


class Board:
    def __init__(self, vid: int | None = None, pid: int | None = None,
                 serial: str | None = None, index: int | None = None) -> None:
        self.dev = hid.device()
        path, self.serial = find_vial_path(vid, pid, serial, index)
        try:
            self.dev.open_path(path)
        except OSError as exc:
            raise SystemExit(
                f"found the keyboard ({self.serial}) but could not open it ({exc}).\n"
                "   the raw-HID interface is exclusive — qmk-state-daemon or the Vial GUI\n"
                "   probably holds it. Try `just daemon-pause` first, then `just daemon-resume`."
            )

    def close(self) -> None:
        self.dev.close()

    def protocol(self) -> tuple[int, bytes]:
        r = send(self.dev, struct.pack("<BBB", CMD_VIAL_PREFIX, VIAL_GET_KEYBOARD_ID, 0))
        return int.from_bytes(r[0:4], "little"), r[4:12]

    def fw_version(self) -> str:
        r = send(self.dev, struct.pack("<BB", SVAL_PREFIX, SVAL_GET_FW_VERSION))
        return bytes(r).split(b"\x00")[0].decode("ascii", "replace")

    def entry_counts(self) -> tuple[int, int, int, int]:
        r = send(self.dev, struct.pack("<BBBB", CMD_VIAL_PREFIX, VIAL_DYNAMIC_ENTRY_OP, ENTRY_GET_COUNT, 0))
        return r[0], r[1], r[2], r[3]

    def combo(self, idx: int) -> tuple[list[int], int]:
        r = send(self.dev, struct.pack("<BBBB", CMD_VIAL_PREFIX, VIAL_DYNAMIC_ENTRY_OP, ENTRY_COMBO_GET, idx))
        if r[0] != 0:
            raise SystemExit(f"combo {idx} read failed (status={r[0]})")
        # vial_combo_entry_t { uint16_t input[4]; uint16_t output; }, native LE at msg[1:11]
        raw = r[1:11]
        words = [int.from_bytes(raw[i:i + 2], "little") for i in range(0, 10, 2)]
        return words[:4], words[4]

    def keycode(self, layer: int, row: int, col: int) -> int:
        r = send(self.dev, struct.pack("<BBBB", VIA_ID_GET_KEYCODE, layer, row, col))
        return (r[4] << 8) | r[5]


def hand_of(row: int) -> str:
    return "L-thumb" if row == 0 else "R-thumb" if row == 5 else ("L" if row < 5 else "R")


def show(board: Board, layer: int, only_tap_hold: bool) -> None:
    lab = labels()
    print(f"=== live layer {layer} ===")
    for row in range(MATRIX_ROWS):
        cells = []
        for col in range(MATRIX_COLS):
            kc = board.keycode(layer, row, col)
            is_th = R_MOD_TAP_LO <= kc <= R_LAYER_TAP_HI
            if only_tap_hold and not is_th:
                continue
            cells.append(f"{lab.get((row, col), '?')}={decode(kc)}")
        if cells:
            print(f"  {hand_of(row):>7} | " + "  ".join(cells))


QSID_MAP = {
    2: ("combo_term", 2),
    7: ("tapping_term", 2),
    18: ("tap_code_delay", 2),
    19: ("tap_hold_caps_delay", 2),
    20: ("tapping_toggle", 1),
    21: ("magic", 4),
    22: ("permissive_hold", 1),
    23: ("hold_on_other_key_press", 1),
    24: ("retro_tapping", 1),
    25: ("quick_tap_term", 2),
    26: ("chordal_hold", 1),
    27: ("flow_tap_term", 2),
    28: ("flow_tap_shift_delta", 2),
    29: ("flow_tap_shift_min_clamp", 1),
}


def qsid(dev: hid.device, num: int, width: int) -> int:
    r = send(dev, struct.pack("<BBH", CMD_VIAL_PREFIX, 0x0A, num))
    return int.from_bytes(r[1:1 + width], "little")


def fingerprint(board: "Board", layers: list[int]) -> dict:
    version, uid = board.protocol()
    fp = {
        "serial": board.serial,
        "uid": uid.hex(),
        "vial_protocol": version,
        "firmware": board.fw_version(),
        "qsids": {f"{n} ({q})": qsid(board.dev, q, w) for q, (n, w) in sorted(QSID_MAP.items())},
        "layers": {},
        "combos": {},
    }
    for layer in layers:
        cells = {}
        for row in range(MATRIX_ROWS):
            for col in range(MATRIX_COLS):
                kc = board.keycode(layer, row, col)
                if kc not in (0x0000, 0xFFFF):
                    cells[f"r{row}c{col}"] = [kc, decode(kc)]
        fp["layers"][str(layer)] = cells
    _, ncombos, _, _ = board.entry_counts()
    for i in range(ncombos):
        inputs, output = board.combo(i)
        if not any(k not in (0x0000, 0xFFFF) for k in inputs):
            continue
        fp["combos"][str(i)] = {
            "inputs": inputs, "output": output,
            "text": " + ".join(decode(k) for k in inputs if k not in (0x0000, 0xFFFF)) + " -> " + decode(output),
        }
    return fp


def show_diff(a: dict, b: dict) -> int:
    print(f"A  {a['serial']}  {a['firmware']}")
    print(f"B  {b['serial']}  {b['firmware']}")
    if a["firmware"] != b["firmware"]:
        print("\n!! FIRMWARE DIFFERS !!")
    print("\n--- settings ---")
    for k in sorted(set(a["qsids"]) | set(b["qsids"])):
        va, vb = a["qsids"].get(k), b["qsids"].get(k)
        print(f"  {'DIFF' if va != vb else '     '}  {k:<36} A={va}  B={vb}")
    print("\n--- keymap cells ---")
    # A partial fingerprint only holds sampled cells; intersect instead of
    # reporting every cell the complete side has and the partial one lacks.
    partial = bool(a.get("partial") or b.get("partial"))
    if partial:
        print("  (partial fingerprint present — only cells captured on both boards are compared)")
    for layer in sorted(set(a["layers"]) | set(b["layers"])):
        la, lb = a["layers"].get(layer, {}), b["layers"].get(layer, {})
        cells = (set(la) & set(lb)) if partial else (set(la) | set(lb))
        for cell in sorted(cells):
            ca, cb = la.get(cell), lb.get(cell)
            if ca != cb:
                print(f"  L{layer} {cell:<5} A={ca[1] if ca else '—':<18} B={cb[1] if cb else '—'}")
    print("\n--- combos ---")
    idxs = sorted(set(a["combos"]) & set(b["combos"]), key=int) if partial else \
           sorted(set(a["combos"]) | set(b["combos"]), key=int)
    for i in idxs:
        ca, cb = a["combos"].get(i), b["combos"].get(i)
        if ca != cb:
            print(f"  combo[{i}]  A={ca['text'] if ca else '—'}")
            print(f"             B={cb['text'] if cb else '—'}")
    return 0


def show_combos(board: Board) -> None:
    td, combos, ko, ark = board.entry_counts()
    print(f"entry counts: tap_dance={td} combos={combos} key_override={ko} alt_repeat={ark}")
    print("=== live combos ===")
    for i in range(combos):
        inputs, output = board.combo(i)
        trig = [decode(k) for k in inputs if k not in (0x0000, 0xFFFF)]
        if not trig:
            continue
        print(f"  combo[{i}]  {' + '.join(trig)}  ->  {decode(output)} (0x{output:04X})")


def main(argv: Iterable[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--vid", type=lambda s: int(s, 0), default=None)
    p.add_argument("--pid", type=lambda s: int(s, 0), default=None)
    p.add_argument("--serial", default=None, help="substring of the vial: serial, to pick a device")
    p.add_argument("--index", type=int, default=None, help="device index when several are attached")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("info")
    sub.add_parser("combos")
    sp = sub.add_parser("dump")
    sp.add_argument("--layer", type=int, default=0)
    sp = sub.add_parser("taps")
    sp.add_argument("--layer", type=int, default=0)
    sp = sub.add_parser("fingerprint")
    sp.add_argument("--layers", default="0,5,6")
    sp.add_argument("--out", default=None)
    sp = sub.add_parser("diff")
    sp.add_argument("a", type=Path)
    sp.add_argument("b", type=Path)
    sp = sub.add_parser("cell")
    sp.add_argument("--layer", type=int, default=0)
    sp.add_argument("--row", type=int, required=True)
    sp.add_argument("--col", type=int, required=True)
    a = p.parse_args(list(argv) if argv is not None else None)

    if a.cmd == "diff":
        return show_diff(json.loads(a.a.read_text()), json.loads(a.b.read_text()))

    board = Board(a.vid, a.pid, a.serial, a.index)
    try:
        if a.cmd == "fingerprint":
            fp = fingerprint(board, [int(x) for x in a.layers.split(",")])
            text = json.dumps(fp, indent=1, sort_keys=True)
            if a.out:
                Path(a.out).write_text(text + "\n")
                print(f"wrote {a.out}  serial={fp['serial']} fw={fp['firmware']} combos={len(fp['combos'])}")
            else:
                print(text)
            return 0
        if a.cmd == "info":
            version, uid = board.protocol()
            print(f"vial protocol   : {version}")
            print(f"keyboard uid    : {uid.hex()}")
            print(f"flashed firmware: {board.fw_version()}")
        elif a.cmd == "combos":
            show_combos(board)
        elif a.cmd == "dump":
            show(board, a.layer, only_tap_hold=False)
        elif a.cmd == "taps":
            show(board, a.layer, only_tap_hold=True)
        else:
            kc = board.keycode(a.layer, a.row, a.col)
            print(f"layer {a.layer} row{a.row} col{a.col} -> {decode(kc)} (0x{kc:04X})")
    finally:
        board.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
