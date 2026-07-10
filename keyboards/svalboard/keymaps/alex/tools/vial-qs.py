#!/usr/bin/env python3
"""vial-qs: get/set arbitrary Vial QMK-Settings QSIDs over raw HID.

Motivation: Vial GUI only renders QSIDs listed in its bundled
`qmk_settings.json` descriptor. Custom QSIDs added in `quantum/qmk_settings.c`
(such as this keymap's 28=flow_tap_shift_delta and 29=flow_tap_shift_min_clamp)
are invisible to the GUI. This script talks the exact same Vial protocol
directly, so we can read/write custom QSIDs without forking Vial desktop.

Concurrency: close Vial GUI while running this script. The GUI caches all
QSID values at connect time and may write stale values back if you tweak
other sliders after this script has modified a QSID.

Requires: pip install hid  (cython-hidapi wrapper)
"""
from __future__ import annotations

import argparse
import struct
import sys
from typing import Iterable

import hid  # type: ignore

VIAL_SERIAL_MAGIC = "vial:"
RAW_USAGE_PAGE = 0xFF60
RAW_USAGE = 0x61
MSG_LEN = 32

CMD_VIAL_PREFIX = 0xFE
CMD_QMK_SETTINGS_QUERY = 0x09
CMD_QMK_SETTINGS_GET = 0x0A
CMD_QMK_SETTINGS_SET = 0x0B
CMD_QMK_SETTINGS_RESET = 0x0C

# Custom QSIDs for this keymap. Keep in sync with quantum/qmk_settings.{h,c}.
# Width in bytes matches the field type in qmk_settings_t.
KNOWN_QSIDS: dict[int, tuple[str, int]] = {
    28: ("flow_tap_shift_delta", 2),
    29: ("flow_tap_shift_min_clamp", 1),
}


def find_vial_device(vid: int | None = None, pid: int | None = None) -> str:
    """Locate the Vial raw-HID interface path."""
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
    dev.open_path(path)
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
    width = args.width or KNOWN_QSIDS.get(args.qsid, (None, 0))[1]
    if not width:
        raise SystemExit(
            f"qsid {args.qsid} not in KNOWN_QSIDS; supply --width {{1,2,4}}"
        )
    dev = open_device(args)
    try:
        val = qsid_get(dev, args.qsid, width)
    finally:
        dev.close()
    print(val)
    return 0


def cmd_set(args: argparse.Namespace) -> int:
    width = args.width or KNOWN_QSIDS.get(args.qsid, (None, 0))[1]
    if not width:
        raise SystemExit(
            f"qsid {args.qsid} not in KNOWN_QSIDS; supply --width {{1,2,4}}"
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
    print(readback)
    return 0


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

    return p


def main(argv: Iterable[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
