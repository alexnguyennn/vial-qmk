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
- `sketchybar/plugins/qmk_state.sh` — reads JSON, updates items.
- `sketchybar/items/qmk.sh` — bootstraps `qmk_layer` + `qmk_mods`.
- `launchd/com.user.qmk-state-daemon.plist` — auto-start template.
- `justfile` — build/install/launchd/sketchybar recipes.
- `VALIDATION.md` — hardware validation walkthrough.

## Quick start

```bash
just build
just detect               # verify raw-HID enumeration
just run-dry              # foreground, dry-run

just install              # copies binary to ~/.local/bin
just install-launchd      # auto-start via launchd
just install-sketchybar   # copies plugin + item to ~/.config/sketchybar
# then in ~/.config/sketchybar/sketchybarrc add:
#     source "$CONFIG_DIR/items/qmk.sh"
sketchybar --reload
```

Once verified, persist `~/.config/sketchybar/plugins/qmk_state.sh` and
`items/qmk.sh` into chezmoi via the `chezmoi` skill (don't do this
until end-to-end works).

## CLI

- `qmk-state-daemon run [--vid …] [--pid …] [--state-file …] [--sketchybar-event …] [--dry-run]`
- `qmk-state-daemon once …` — read one packet and exit (debug).
- `qmk-state-daemon list` — enumerate raw-HID devices.

Defaults: VID/PID `0x303A:0x4044` (svalboard), state file
`/tmp/qmk_state.json`, event `qmk_state_changed`.

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
