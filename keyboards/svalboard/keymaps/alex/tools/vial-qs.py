#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "hidapi>=0.14",
# ]
# ///
"""vial-qs: FALLBACK direct-HID CLI for Vial QMK-Settings QSIDs.

Prefer the qmk-state-daemon RPC path (`qmk-state-daemon qsid list|get|set`
or `just get 28` from this directory). Use this script only when the
daemon isn't running.

Motivation: Vial GUI only renders QSIDs listed in its bundled
`qmk_settings.json` descriptor. Custom QSIDs added in `quantum/qmk_settings.c`
(such as this keymap's 28=flow_tap_shift_delta and 29=flow_tap_shift_min_clamp)
are invisible to the GUI. This script talks the exact same Vial protocol
directly, so we can read/write custom QSIDs without forking Vial desktop.

Concurrency: stop the daemon (`launchctl bootout gui/$UID/com.user.qmk-state-daemon`)
before running this script — both hold the raw-HID interface exclusively.
Also close Vial GUI while running.

Usage: `uv run --project . vial-qs.py list` (see justfile).
"""
from __future__ import annotations

import argparse
import re
import struct
import sys
from pathlib import Path
from typing import Iterable

try:
    import hid  # type: ignore
except ModuleNotFoundError:  # `names` and `check` work without hidapi installed
    hid = None

VIAL_SERIAL_MAGIC = "vial:"
RAW_USAGE_PAGE = 0xFF60
RAW_USAGE = 0x61
MSG_LEN = 32

CMD_VIAL_PREFIX = 0xFE
CMD_QMK_SETTINGS_QUERY = 0x09
CMD_QMK_SETTINGS_GET = 0x0A
CMD_QMK_SETTINGS_SET = 0x0B
CMD_QMK_SETTINGS_RESET = 0x0C

# Full QSID table for this firmware, generated from quantum/qmk_settings.c `protos[]`
# and the field widths in `qmk_settings_t` (quantum/qmk_settings.h).
#   - plain settings   -> width is the field's sizeof
#   - bit settings     -> 1 byte on the wire regardless of the backing field width
#                         (eeprom_settings_getbit/setbit require maxsz >= 1)
#   - qsid 21 "magic"  -> 4 bytes, it is a uint32 built from keymap_config
# Run `vial-qs.py check` (or `just py-check`) to verify this against the firmware source.
KNOWN_QSIDS: dict[int, tuple[str, int]] = {
    1: ("grave_esc_override", 1),
    2: ("combo_term", 2),
    3: ("auto_shift", 1),
    4: ("auto_shift_timeout", 2),
    5: ("osk_tap_toggle", 1),
    6: ("osk_timeout", 2),
    7: ("tapping_term", 2),
    9: ("mousekey_delay", 2),
    10: ("mousekey_interval", 2),
    11: ("mousekey_move_delta", 2),
    12: ("mousekey_max_speed", 2),
    13: ("mousekey_time_to_max", 2),
    14: ("mousekey_wheel_delay", 2),
    15: ("mousekey_wheel_interval", 2),
    16: ("mousekey_wheel_max_speed", 2),
    17: ("mousekey_wheel_time_to_max", 2),
    18: ("tap_code_delay", 2),
    19: ("tap_hold_caps_delay", 2),
    20: ("tapping_toggle", 1),
    21: ("magic", 4),
    22: ("permissive_hold", 1),          # bit 0 of tapping_v2
    23: ("hold_on_other_key_press", 1),  # bit 1 of tapping_v2
    24: ("retro_tapping", 1),            # bit 2 of tapping_v2 - inert without RETRO_SHIFT
    25: ("quick_tap_term", 2),
    26: ("chordal_hold", 1),             # bit 3 of tapping_v2
    27: ("flow_tap_term", 2),
    28: ("flow_tap_shift_delta", 2),
    29: ("flow_tap_shift_min_clamp", 1),
}

# QSIDs worth printing on `dump`, in reading order.
INTERESTING_QSIDS = (2, 7, 20, 22, 23, 24, 25, 26, 27, 28, 29)

REPO_ROOT = Path(__file__).resolve().parents[5]


def table_from_firmware() -> dict[int, tuple[str, int]] | None:
    """Rebuild KNOWN_QSIDS from the firmware sources, for drift checking."""
    try:
        c = (REPO_ROOT / "quantum/qmk_settings.c").read_text()
        h = (REPO_ROOT / "quantum/qmk_settings.h").read_text()
    except OSError as exc:
        print(f"cannot read firmware sources: {exc}", file=sys.stderr)
        return None

    sizes = {name: (4 if typ == "uint32_t" else 2 if typ == "uint16_t" else 1)
             for typ, name in re.findall(r"\b(uint\d+_t)\s+(\w+);", h)}

    out: dict[int, tuple[str, int]] = {}
    for macro, decl in re.findall(r"DECLARE_((?:STATIC_)?(?:BIT)?SETTING(?:_NOTIFY)?)\(([^)]*)\)", c):
        parts = [p.strip() for p in decl.split(",")]
        if not parts or not parts[0].isdigit():
            continue
        qsid = int(parts[0])
        if "STATIC" not in macro:
            continue  # custom get/set pair, not a QS field — see the magic special case below
        field = parts[1]
        out[qsid] = (field, 1 if "BIT" in macro else sizes.get(field, 0))
    # qsid 21 is DECLARE_SETTING(21, magic_settings_get, magic_settings_set): a uint32
    # assembled from keymap_config, not a member of qmk_settings_t.
    if "magic_settings_get" in c:
        out[21] = ("magic", 4)
    return out


def find_vial_device(vid: int | None = None, pid: int | None = None) -> str:
    """Locate the Vial raw-HID interface path."""
    if hid is None:
        raise SystemExit(
            "hidapi not available in this environment — run via uv (`just py-get ...`)\n"
            "   or: uv sync && uv run --project . vial-qs.py ...\n"
            "   offline subcommands (names, check) need no device."
        )
    matches: list[dict] = []
    for info in hid.enumerate(vid or 0, pid or 0):
        if info.get("usage_page") != RAW_USAGE_PAGE or info.get("usage") != RAW_USAGE:
            continue
        serial = info.get("serial_number") or ""
        if VIAL_SERIAL_MAGIC in serial:
            matches.append(info)
    if not matches:
        raise SystemExit(
            "no vial keyboard found on raw-hid interface "
            f"(usage_page=0x{RAW_USAGE_PAGE:04x} usage=0x{RAW_USAGE:02x})"
        )
    if len(matches) > 1:
        print(
            f"warning: {len(matches)} vial devices found; using the first",
            file=sys.stderr,
        )
    return matches[0]["path"]


def send(dev: hid.device, payload: bytes) -> bytes:
    if len(payload) > MSG_LEN:
        raise ValueError(f"payload too large: {len(payload)} > {MSG_LEN}")
    payload = payload + b"\x00" * (MSG_LEN - len(payload))
    # HID report id 0x00 prefix.
    dev.write(b"\x00" + payload)
    resp = dev.read(MSG_LEN, timeout_ms=500)
    if not resp:
        raise SystemExit("no response from device (timeout)")
    return bytes(resp)


def qsid_get(dev: hid.device, qsid: int, width: int) -> int:
    resp = send(
        dev, struct.pack("<BBH", CMD_VIAL_PREFIX, CMD_QMK_SETTINGS_GET, qsid)
    )
    if resp[0] != 0:
        raise SystemExit(f"GET qsid={qsid} failed (status={resp[0]})")
    raw = resp[1 : 1 + width]
    if len(raw) < width:
        raise SystemExit(f"short response for qsid={qsid}: {len(raw)} < {width}")
    return int.from_bytes(raw, "little")


def qsid_set(dev: hid.device, qsid: int, value: int, width: int) -> None:
    value_bytes = value.to_bytes(width, "little")
    resp = send(
        dev,
        struct.pack("<BBH", CMD_VIAL_PREFIX, CMD_QMK_SETTINGS_SET, qsid)
        + value_bytes,
    )
    if resp[0] != 0:
        raise SystemExit(f"SET qsid={qsid}={value} failed (status={resp[0]})")


def qsid_list(dev: hid.device) -> list[int]:
    supported: set[int] = set()
    cursor = 0
    seen_end = False
    while not seen_end:
        resp = send(
            dev,
            struct.pack("<BBH", CMD_VIAL_PREFIX, CMD_QMK_SETTINGS_QUERY, cursor),
        )
        new_max = cursor
        for i in range(0, len(resp), 2):
            q = int.from_bytes(resp[i : i + 2], "little")
            if q == 0xFFFF:
                seen_end = True
                break
            supported.add(q)
            if q > new_max:
                new_max = q
        if new_max == cursor:
            # nothing new returned; avoid infinite loop
            break
        cursor = new_max
    return sorted(supported)


def open_device(args: argparse.Namespace) -> hid.device:
    path = find_vial_device(args.vid, args.pid)
    dev = hid.device()
    try:
        dev.open_path(path)
    except OSError as exc:
        # The device was enumerated but the handle could not be opened. On macOS that
        # almost always means another process already owns the raw-HID interface.
        raise SystemExit(
            f"found the keyboard but could not open it ({exc}).\n"
            "   the raw-HID interface is exclusive — most likely qmk-state-daemon or the\n"
            "   Vial GUI holds it. Try:  just daemon-pause   …then retry, and\n"
            "   just daemon-resume  afterwards. Or skip direct HID entirely with the\n"
            "   daemon recipes:  just get 2 / just set 2 120"
        )
    return dev


def cmd_list(args: argparse.Namespace) -> int:
    dev = open_device(args)
    try:
        qsids = qsid_list(dev)
    finally:
        dev.close()
    for q in qsids:
        name, width = KNOWN_QSIDS.get(q, ("?", 0))
        width_str = f"{width}B" if width else "?"
        print(f"  {q:>3}  width={width_str}  {name}")
    return 0


def cmd_get(args: argparse.Namespace) -> int:
    name, known_width = KNOWN_QSIDS.get(args.qsid, (None, 0))
    width = args.width or known_width
    if not width:
        raise SystemExit(
            f"qsid {args.qsid} unknown to this tool and no --width given.\n"
            f"   named QSIDs: {', '.join(str(q) for q in sorted(KNOWN_QSIDS))}\n"
            f"   or ask the board which QSIDs it supports: vial-qs.py list\n"
            f"   or read one blindly: vial-qs.py get {args.qsid} --width {{1,2,4}}"
        )
    dev = open_device(args)
    try:
        val = qsid_get(dev, args.qsid, width)
    finally:
        dev.close()
    label = f"  # {name}" if name else ""
    print(f"{val}{label}")
    return 0


def cmd_set(args: argparse.Namespace) -> int:
    name, known_width = KNOWN_QSIDS.get(args.qsid, (None, 0))
    width = args.width or known_width
    if not width:
        raise SystemExit(
            f"qsid {args.qsid} unknown to this tool and no --width given.\n"
            f"   named QSIDs: {', '.join(str(q) for q in sorted(KNOWN_QSIDS))}\n"
            f"   refusing to guess a width when writing."
        )
    if args.value < 0 or args.value >= (1 << (width * 8)):
        raise SystemExit(
            f"value {args.value} out of range for width={width}B"
        )
    dev = open_device(args)
    try:
        qsid_set(dev, args.qsid, args.value, width)
        readback = qsid_get(dev, args.qsid, width)
    finally:
        dev.close()
    if readback != args.value:
        print(
            f"warning: readback ({readback}) != set value ({args.value})",
            file=sys.stderr,
        )
    print(f"{readback}  # {name}" if name else str(readback))
    return 0


def cmd_width(args: argparse.Namespace) -> int:
    """Print the wire width of a QSID. Offline — used by the justfile to fill in --width."""
    name, width = KNOWN_QSIDS.get(args.qsid, (None, 0))
    if not width:
        print(f"unknown qsid {args.qsid}", file=sys.stderr)
        return 1
    if args.verbose:
        print(f"{width}  # {name}")
    else:
        print(width)
    return 0


def cmd_names(_: argparse.Namespace) -> int:
    """Print the built-in table. Works with no keyboard attached."""
    print("  qsid  width  name                       (built-in table; no keyboard needed)")
    for q, (name, width) in sorted(KNOWN_QSIDS.items()):
        print(f"  {q:>3}  {width}B     {name}")
    return 0


def cmd_dump(args: argparse.Namespace) -> int:
    """Read every interesting QSID in one device session."""
    dev = open_device(args)
    try:
        for q in (sorted(KNOWN_QSIDS) if args.all else INTERESTING_QSIDS):
            name, width = KNOWN_QSIDS[q]
            try:
                val = qsid_get(dev, q, width)
            except SystemExit as exc:
                print(f"  {q:>3}  {name:<26} -        ({exc})")
                continue
            print(f"  {q:>3}  {name:<26} {val}")
    finally:
        dev.close()
    return 0


def cmd_check(_: argparse.Namespace) -> int:
    """Diff KNOWN_QSIDS against the firmware sources, so this table cannot go stale silently."""
    fw = table_from_firmware()
    if fw is None:
        return 2
    drift = 0
    for q in sorted(set(fw) | set(KNOWN_QSIDS)):
        want, have = fw.get(q), KNOWN_QSIDS.get(q)
        if want is None:
            print(f"  {q:>3}  in this tool only: {have[0]}"); drift += 1
        elif have is None:
            print(f"  {q:>3}  missing from this tool: {want[0]} ({want[1]}B)"); drift += 1
        elif have[1] != want[1]:
            print(f"  {q:>3}  width mismatch: tool={have[1]}B firmware={want[1]}B ({want[0]})"); drift += 1
    if not drift:
        print(f"  in sync with {REPO_ROOT / 'quantum/qmk_settings.c'} ({len(fw)} QSIDs)")
    return 1 if drift else 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--vid", type=lambda s: int(s, 0), default=None)
    p.add_argument("--pid", type=lambda s: int(s, 0), default=None)
    sub = p.add_subparsers(dest="cmd", required=True)

    sp = sub.add_parser("list", help="list all supported QSIDs")
    sp.set_defaults(func=cmd_list)

    sp = sub.add_parser("get", help="read a QSID value")
    sp.add_argument("qsid", type=lambda s: int(s, 0))
    sp.add_argument("--width", type=int, choices=(1, 2, 4), default=None)
    sp.set_defaults(func=cmd_get)

    sp = sub.add_parser("set", help="write a QSID value")
    sp.add_argument("qsid", type=lambda s: int(s, 0))
    sp.add_argument("value", type=lambda s: int(s, 0))
    sp.add_argument("--width", type=int, choices=(1, 2, 4), default=None)
    sp.set_defaults(func=cmd_set)

    sp = sub.add_parser("names", help="print the built-in QSID table (no keyboard needed)")
    sp.set_defaults(func=cmd_names)

    sp = sub.add_parser("width", help="print a QSID's byte width (no keyboard needed)")
    sp.add_argument("qsid", type=lambda s: int(s, 0))
    sp.add_argument("-v", "--verbose", action="store_true")
    sp.set_defaults(func=cmd_width)

    sp = sub.add_parser("dump", help="read every interesting QSID in one session")
    sp.add_argument("--all", action="store_true", help="include mousekey/one-shot/auto-shift ids")
    sp.set_defaults(func=cmd_dump)

    sp = sub.add_parser("check", help="diff the QSID table against quantum/qmk_settings.c")
    sp.set_defaults(func=cmd_check)

    return p


def main(argv: Iterable[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
