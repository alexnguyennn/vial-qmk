#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["hidapi>=0.14"]
# ///
"""sval-state: watch the firmware's live layer/mod snapshot in real time.

The alex keymap pushes a 32-byte 0xAB qmk-state broadcast whenever layers,
default layer, real/weak/one-shot mods or caps-word change, plus a 5 s
heartbeat (see keymap.c, "QMK state broadcast"). This tool decodes that stream
so you can watch a power-on and see *exactly* when state gets latched — the
only way to tell "the matrix thinks keys are held" apart from "a combo latched
a mod" is to see the transition happen.

The qmk-state-daemon owns raw HID exclusively; stop it first:
    just daemon-pause    ... run this ...    just daemon-resume

Usage:
  uv run --project . sval-state.py             # 10 s
  uv run --project . sval-state.py 30          # 30 s (reset the board during it)
  uv run --project . sval-state.py 0           # until Ctrl-C
  just svalboard boot-watch 20                 # same thing, with a reset prompt
"""
from __future__ import annotations

import sys
import time

import hid

# svalboard: vendor 0x303A product 0x4044 ("lightly")
VID, PID = 0x303A, 0x4044
MSG_ID = 0xAB
PKT_LEN = 32

REASONS = {
    0x01: "LAYER",
    0x02: "DEFAULT",
    0x04: "MODS",
    0x08: "OSM",
    0x10: "INITIAL/HB",
    0x20: "CAPSWORD",
}
MOD_BITS = [
    (0x01, "LCTL"), (0x02, "LSFT"), (0x04, "LALT"), (0x08, "LGUI"),
    (0x10, "RCTL"), (0x20, "RSFT"), (0x40, "RALT"), (0x80, "RGUI"),
]


def mods(byte: int) -> str:
    return "+".join(name for bit, name in MOD_BITS if byte & bit) or "-"


def layer_bits(bits: int) -> str:
    return " ".join(str(i) for i in range(32) if bits & (1 << i)) or "-"


def decode(r: list[int]) -> dict:
    return {
        "reason": r[2],
        "top": r[3],
        "dflt": r[4],
        "layers": int.from_bytes(r[5:9], "little"),
        "mods": r[9],
        "weak": r[10],
        "osm": r[11],
        "osm_lock": r[12],
        "caps_word": r[13],
    }


def main() -> int:
    dur = float(sys.argv[1]) if len(sys.argv) > 1 else 10.0
    dev = hid.device()
    dev.open(VID, PID)
    dev.set_nonblocking(1)
    t0 = time.time()
    prev = None
    print(f"# listening for 0xAB state broadcasts for {dur:.0f}s"
          + ("  (reset/re-seat the board now)" if dur > 10 else ""))
    print("# t       reason          top dflt layers          mods / weak / osm")
    try:
        while dur <= 0 or (time.time() - t0) < dur:
            pkt = dev.read(PKT_LEN)
            if not pkt or pkt[0] != MSG_ID:
                continue
            s = decode(pkt)
            tag = "|".join(k for v, k in REASONS.items() if s["reason"] & v) or hex(s["reason"])
            line = (f"{time.time() - t0:7.2f}s {tag:>12}  {s['top']:3d} {s['dflt']:4d} "
                    f"[{layer_bits(s['layers'])}]  mods={mods(s['mods'])} "
                    f"weak={mods(s['weak'])} osm={mods(s['osm'])} lock={mods(s['osm_lock'])} "
                    f"caps_word={s['caps_word']}")
            if line[9:] != prev:
                print(line, flush=True)
                prev = line[9:]
    finally:
        dev.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
