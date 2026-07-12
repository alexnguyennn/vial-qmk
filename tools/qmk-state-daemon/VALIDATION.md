# QMK State Daemon — Hardware Validation Guide

Manual test steps to validate the raw-HID broadcast pipeline end-to-end
after flashing the combined firmware (Workstream A flow-tap fix +
Workstream B state broadcast) from branch `feature/initial-customise`.

Daemon lives on branch `feature/initial-customise`.

## 0. Prereqs

- Both svalboard halves powered and connected via USB.
- `mise` active (rust 1.93.1 available).
- Vial GUI **closed** — it may hold the raw-HID interface exclusively
  on macOS.

## 1. Flash the firmware

UF2s live at the repo root, freshly built for both halves:

```
svalboard_trackball_pmw3389_left_alex.uf2
svalboard_trackball_pmw3389_right_alex.uf2
```

Steps:
1. Double-tap the RESET button on the left half.
2. Drag `svalboard_trackball_pmw3389_left_alex.uf2` onto the
   `RPI-RP2` volume.
3. Wait for remount, then repeat for the right half with the
   right UF2.

## 2. Build the daemon (once)

```bash
cd /Users/alex/bench/cfg/vial-qmk
git switch feature/initial-customise
cargo build --release --manifest-path tools/qmk-state-daemon/Cargo.toml
```

Binary: `tools/qmk-state-daemon/target/release/qmk-state-daemon`.

## 3. Verify raw-HID enumeration

```bash
tools/qmk-state-daemon/target/release/qmk-state-daemon list
```

Expected output includes a line for the svalboard:

```
303A:4044  FF60/0061       vial:xxxxxxxxxxxx    svalboard trackball pmw3389
```

Troubleshooting:
- Nothing shown: run `system_profiler SPUSBDataType | grep -A5 svalboard`
  to confirm the OS sees the device.
- Wrong VID/PID: pass `--vid 0xXXXX --pid 0xXXXX` to subsequent
  commands.
- Only `usage_page` other than `FF60`: firmware didn't build raw HID.
  Check `qmk info -kb svalboard/trackball/pmw3389/left -km alex | grep -i raw`.

## 4. Capture packets (dry-run)

```bash
tools/qmk-state-daemon/target/release/qmk-state-daemon run \
  --state-file /tmp/qmk_state.json --dry-run
```

This runs until interrupted. Trigger options:
- Wait up to 5 seconds — firmware fires a heartbeat.
- Press any key that changes layer or mods (e.g. tap `MO(NAS)`,
  hold `LSFT_T(KC_V)`).
- Unplug/replug USB — fires initial snapshot on connect.

On success the command keeps running silently. Inspect the JSON from
another terminal:

```bash
cat /tmp/qmk_state.json | jq
```

Expected shape (idle):

```json
{
  "type": "state",
  "version": 1,
  "reason": 16,
  "reason_flags": ["initial"],
  "top_layer": 0,
  "top_layer_name": "BASE",
  "default_layer": 0,
  "default_layer_name": "BASE",
  "layer_state": 1,
  "real_mods": 0,
  "weak_mods": 0,
  "oneshot_mods": 0,
  "locked_mods": 0,
  "mods_letters": "",
  "mods_state": "none"
}
```

## 5. Run continuously and watch state change

Terminal A (daemon):

```bash
tools/qmk-state-daemon/target/release/qmk-state-daemon run \
  --state-file /tmp/qmk_state.json --dry-run
```

Terminal B (watcher):

```bash
while true; do clear; cat /tmp/qmk_state.json | jq; sleep 0.2; done
```

Exercise the keyboard and confirm the JSON updates:

| Action                                    | Expected change                                     |
| ----------------------------------------- | --------------------------------------------------- |
| Tap `MO(NAS)`                             | `top_layer_name` → `NAS`, `reason_flags` has `layer`|
| Hold `LSFT_T(KC_V)`                       | `mods_letters` → `S`, `mods_state` → `held`         |
| Fire a `CSG` combo (e.g. `csg_p`)         | `mods_letters` → `CSG`, `mods_state` → `held`       |
| Release everything, wait 5s               | Heartbeat packet with `reason_flags` `["initial"]`  |
| Switch to `FUNC` layer                    | `top_layer_name` → `FN`                             |
| One-shot mod (`OSM(KC_LSFT)` if bound)    | `mods_state` → `osm`                                |

Stop the foreground daemon with Ctrl-C before continuing.

## 6. Validate config and RPC

```bash
tools/qmk-state-daemon/target/release/qmk-state-daemon write-default-config \
  --path /tmp/qmk-state-daemon.toml --force
tools/qmk-state-daemon/target/release/qmk-state-daemon run \
  --config /tmp/qmk-state-daemon.toml \
  --state-file /tmp/qmk_state.json \
  --socket /tmp/qmk-state-daemon.sock \
  --dry-run
```

From another terminal:

```bash
tools/qmk-state-daemon/target/release/qmk-state-daemon ping \
  --socket /tmp/qmk-state-daemon.sock
tools/qmk-state-daemon/target/release/qmk-state-daemon qsid get 28 \
  --socket /tmp/qmk-state-daemon.sock
tools/qmk-state-daemon/target/release/qmk-state-daemon reload-config \
  --socket /tmp/qmk-state-daemon.sock
```

Expected:

- `ping` returns `ok: true`.
- QSID 28 returns the configured flow-tap shift delta, usually `25`.
- `reload-config` returns `ok: true` and lists `vid`, `pid`, `socket` as restart-required fields.

## 7. Service validation

macOS:

```bash
cd tools/qmk-state-daemon
just install-launchd
qmk-state-daemon ping
qmk-state-daemon qsid get 28
sketchybar --reload
```

Linux/systemd user service:

```bash
cd tools/qmk-state-daemon
just install-udev-rule 303a 4044   # only if hidraw access fails
just install-systemd-user
qmk-state-daemon ping
qmk-state-daemon qsid get 28
```

## 8. Known gotchas

- **Vial GUI running**: quit it before running the daemon; it holds the
  raw-HID interface exclusively on some OSes.
- **VID/PID mismatch**: pass `--vid`/`--pid` explicitly; discover with
  the `list` subcommand.
- **macOS Input Monitoring permission**: if hidapi errors with
  "Permission denied", grant Input Monitoring to your terminal in
  System Settings → Privacy & Security → Input Monitoring.
- **RAW_ENABLE missing**: unlikely with VIA/VIAL enabled, but verify
  with `qmk info -kb svalboard/trackball/pmw3389/left -km alex | grep raw`
  (should show `raw: True`).
- **No packets ever**: check daemon stderr; add `RUST_LOG=debug` if
  logging is ever wired in. For now, `--dry-run` prints nothing on
  success — packet reception is silent.
- **Linux hidraw permission denied**: run `just install-udev-rule 303a 4044`,
  unplug/replug the keyboard, then retry `just detect`.

## 9. Next steps after validation

Once steps 3–5 pass on hardware:
- Keep using daemon RPC for QSIDs 28/29.
- Use launchd on macOS or systemd user service on Linux for daily use.
- Persist desktop integration config only after end-to-end works.

Report which step fails (if any) and paste the JSON / error output.
