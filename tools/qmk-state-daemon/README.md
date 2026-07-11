# qmk-state-daemon

Rust daemon that reads QMK raw-HID state packets from a Vial-enabled
keyboard (svalboard by default), decodes them to JSON, atomically writes
to `/tmp/qmk_state.json`, and triggers a sketchybar event.

Push-only: firmware broadcasts on layer/mod change plus a 5s heartbeat.
Daemon dedupes identical payloads so idle heartbeats don't spam
sketchybar.

## Layout

- `src/` — library + binary (see `PLAN_flow_tap_shift.md` for
  Workstream B architecture: `PacketHandler` + `Registry`, transport
  abstraction, atomic writer, notifier trait).
- `launchd/com.user.qmk-state-daemon.plist` — auto-start template.
- `justfile` — build/install/launchd recipes.
- `VALIDATION.md` — hardware validation walkthrough.

Sketchybar items live in the user's sketchybar Lua config
(`~/.config/sketchybar/lua/items/qmk-layer.lua` and `qmk-mods.lua`),
persisted via chezmoi. They subscribe to the `qmk_state_changed`
event and read `event.top_layer_name`, `event.mods_letters`,
`event.mods_state`, `event.default_layer_name` — all populated
directly by the daemon via `sketchybar --trigger EVENT k=v k=v`, so
no jq / JSON parsing is needed on the Lua side.

## Quick start

```bash
just build
just detect               # verify raw-HID enumeration
just run-dry              # foreground, dry-run

just install              # copies binary to ~/.local/bin
just install-launchd      # auto-start via launchd
# Sketchybar lua items already ship in ~/.config/sketchybar/lua/items/
sketchybar --reload
```

Once verified, persist the lua items and any dotfile changes via
chezmoi (`chezmoi re-add`). Do not persist before end-to-end works.

## CLI

Daemon:
- `qmk-state-daemon run [--vid …] [--pid …] [--state-file …] [--sketchybar-event …] [--socket …] [--dry-run]`
- `qmk-state-daemon list` — enumerate raw-HID devices.

RPC clients (require the daemon to be running):
- `qmk-state-daemon ping [--socket …]` — health check.
- `qmk-state-daemon qsid list` — enumerate custom QSIDs on the keyboard.
- `qmk-state-daemon qsid get <qsid> [--width 1|2|4]` — read a QSID.
- `qmk-state-daemon qsid set <qsid> <value> [--width 1|2|4]` — write a QSID.

Widths for known QSIDs (see `src/vial_qsid.rs::known_qsids`) are
resolved automatically. Custom QSIDs need `--width`.

Defaults: VID/PID `0x303A:0x4044` (svalboard), state file
`/tmp/qmk_state.json`, event `qmk_state_changed`, socket
`/tmp/qmk-state-daemon.sock`.

## Architecture: one HID owner, socket-based RPC

The daemon is the sole owner of the raw-HID interface. Both the push
state pipeline (`0xAB` packets) and the Vial custom-QSID protocol
(`0xFE 0x09/0x0A/0x0B`) share that one connection.

Reader/writer are serialized in a single broker thread:
1. `rpc::spawn_broker` owns the HID transport.
2. Each loop iteration either services a pending outbound QSID
   request (write + wait for response, dropping interleaved state
   packets) or performs a normal read (routing `0xAB` packets to the
   state pipeline).
3. Only one QSID RPC is ever in flight, so no response-correlation
   ID is needed.

The socket server (`rpc::Server`) binds a Unix domain socket, accepts
line-delimited JSON requests, and forwards them to the broker via an
`mpsc` channel. Client commands (`qsid list|get|set`, `ping`) connect,
send one request, read one response.

## JSON payload

```json
{
  "type": "state",
  "version": 1,
  "reason": 5,
  "reason_flags": ["layer", "mods"],
  "top_layer": 4,
  "top_layer_name": "NAS",
  "default_layer": 0,
  "default_layer_name": "BASE",
  "layer_state": 17,
  "real_mods": 3,
  "weak_mods": 0,
  "oneshot_mods": 0,
  "locked_mods": 0,
  "mods_letters": "CS",
  "mods_state": "held"
}
```

Dedup ignores `reason` + `reason_flags`; everything else drives the
change signal.

## Adding a new HID handler

1. Implement `PacketHandler` for your msg id in `src/handlers/`.
2. Register with `Registry::builder().handler(YourHandler::new())`.
3. Add a fixture-based unit test.

Dispatch code doesn't change.

## Troubleshooting

- `list` shows nothing → keyboard not enumerated; check
  `system_profiler SPUSBDataType | grep -A5 svalboard`.
- Vial GUI running → close it; it may hold the raw-HID interface.
- macOS "Permission denied" from hidapi → grant Input Monitoring to
  your terminal in System Settings → Privacy & Security.
- launchd not restarting → `just launchd-status`, then
  `just launchd-logs`.
