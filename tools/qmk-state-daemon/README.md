# qmk-state-daemon

Rust daemon that reads QMK raw-HID state packets from a Vial-enabled
keyboard (svalboard by default), decodes them to JSON, atomically writes
to a state file, triggers a configured event sink, and serves local QSID
RPCs over a Unix socket.

Push-only: firmware broadcasts on layer/mod change plus a 5s heartbeat.
Daemon dedupes identical payloads so idle heartbeats don't spam
sketchybar.

## Layout

- `src/` — library + binary (`PacketHandler` + `Registry`, transport
  abstraction, atomic writer, `EventSink` trait).
- `launchd/com.user.qmk-state-daemon.plist` — auto-start template.
- `systemd/qmk-state-daemon.service` — Linux user-service template.
- `examples/i3status-rust/` — generic command-sink example for i3status-rust.
- `justfile` — build/install/launchd/systemd/udev recipes.
- `VALIDATION.md` — hardware validation walkthrough.
- `RUNBOOK.md` — recovery steps for stale sketchybar / stuck QSID RPC / launchd issues.

Sketchybar items live in the user's sketchybar Lua config
(`~/.config/sketchybar/lua/items/qmk-layer.lua` and `qmk-mods.lua`),
persisted via chezmoi. They subscribe to the `qmk_state_changed`
event and read `event.top_layer_name`, `event.mods_letters`,
`event.mods_state`, `event.default_layer_name` — all populated
directly by the daemon via `sketchybar --trigger EVENT k=v k=v`, so
no jq / JSON parsing is needed on the Lua side.

## Quick start

macOS / sketchybar:

```bash
just build
just detect               # verify raw-HID enumeration
just run-dry              # foreground, dry-run

just install              # copies binary to ~/.local/bin
just install-launchd      # auto-start via launchd
# Sketchybar lua items already ship in ~/.config/sketchybar/lua/items/
sketchybar --reload
```

Linux / systemd user service:

```bash
just build
just detect
just install-udev-rule 303a 4044   # if hidraw permission is denied
just install-systemd-user
just systemd-status
```

The Linux default config uses the generic command sink and writes a
sample event log. For i3status-rust, prefer the `custom_dbus` example in
`examples/i3status-rust/config-example.toml` plus daemon config in
`examples/i3status-rust/qmk-state-daemon-dbus.toml`.

Once verified, persist the lua items and any dotfile changes via
chezmoi (`chezmoi re-add`). Do not persist before end-to-end works.

## Temporarily Using Vial Or Direct HID Tools

The daemon and Vial/direct-HID tools all want the same raw-HID
interface. They do fight each other: whichever process owns the raw-HID
handle first wins, and the others will fail to open it or behave
erratically.

Important consequences:

- `qmk-state-daemon` does **not** currently have an in-process
  "suspend" mode.
- Pausing/resuming happens at the **launchd layer**.
- The Python fallback `vial-qs.py` only works while the daemon is
  stopped.
- Vial GUI should also be used while the daemon is stopped.

Recommended handoff:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just pause-launchd

# now use Vial GUI, or from
# ~/bench/cfg/vial-qmk/keyboards/svalboard/keymaps/alex/tools
# run: just py-list / just py-get 28 / just py-set 28 40

just resume-launchd
sketchybar --reload
```

Equivalent shortcuts from the keymap tools directory:

```bash
cd ~/bench/cfg/vial-qmk/keyboards/svalboard/keymaps/alex/tools
just daemon-pause
# use Vial or py-* recipes
just daemon-resume
```

Note: Vial GUI cannot render custom QSIDs 28/29, so use the daemon RPC
(`qmk-state-daemon qsid ...`) or the Python fallback for flow-tap delta
and clamp.

## CLI

Daemon:
- `qmk-state-daemon run [--config …] [--vid …] [--pid …] [--state-file …] [--event-name …] [--sink sketchybar|command|null] [--command …] [--socket …] [--dry-run]`
- `qmk-state-daemon list` — enumerate raw-HID devices.
- `qmk-state-daemon config-path` — print the default config path.
- `qmk-state-daemon write-default-config [--path …] [--force]` — create a starter config.

RPC clients (require the daemon to be running):
- `qmk-state-daemon ping [--socket …]` — health check.
- `qmk-state-daemon reload-config [--socket …]` — reload sink, event name, and state-file path.
- `qmk-state-daemon qsid list` — enumerate custom QSIDs on the keyboard.
- `qmk-state-daemon qsid get <qsid> [--width 1|2|4]` — read a QSID.
- `qmk-state-daemon qsid set <qsid> <value> [--width 1|2|4]` — write a QSID.

Widths for known QSIDs (see `src/vial_qsid.rs::known_qsids`) are
resolved automatically. Custom QSIDs need `--width`.

Defaults: VID/PID `0x303A:0x4044` (svalboard), event
`qmk_state_changed`, config
`${XDG_CONFIG_HOME:-~/.config}/qmk-state-daemon/config.toml`, state file
`${XDG_STATE_HOME:-~/.local/state}/qmk-state-daemon/state.json`, and
socket `${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.sock`.

Config example:

```toml
vid = "0x303A"
pid = "0x4044"
# state_file = "auto"
# socket = "auto"

[sink]
kind = "command"
event = "qmk_state_changed"
program = ["/path/to/qmk-state-to-file.sh"]
```

`reload-config` applies sink, event, and state-file changes without
restarting. VID, PID, and socket changes still require service restart.

## Event Sinks

Sinks receive deduped state changes after the JSON file is written.

- `sketchybar` runs `sketchybar --trigger EVENT k=v ...` and is only available on macOS.
- `command` runs an arbitrary command with state in environment variables.
- `null` writes JSON only and emits no external event.

`command` supports either one command or several commands:

```toml
[sink]
kind = "command"
program = ["/path/to/program", "arg"]

# Or:
commands = [
  ["program-one", "arg"],
  ["program-two", "arg"],
]
```

All commands receive the same event environment and run in order. If one
command exits non-zero, the sink reports the failure and stops the
sequence for that event.

Command args can reference event fields without using a shell:

```toml
commands = [
  ["printf", "layer={top_layer_name} mods={mods_letters}\n"],
]
```

Supported templates include `{top_layer_name}`, `{default_layer_name}`,
`{mods_letters}`, `{mods_state}`, `{event_name}`, `{state_json}`,
`{state_payload_json}`, and their uppercase environment names such as
`{QMK_TOP_LAYER_NAME}`.

Command sink environment:

- `QMK_EVENT_NAME` — configured event name.
- `QMK_STATE_JSON` — path to the current state JSON file.
- `QMK_STATE_PAYLOAD_JSON` — full decoded JSON payload for this event.
- `QMK_TOP_LAYER_NAME`, `QMK_MODS_LETTERS`, `QMK_MODS_STATE`, and other scalar event fields.

## Linux Systemd Setup

Use this on Regolith/Ubuntu with a Sway session, or any Linux desktop
with user systemd.

1. Build, install, and create the user service:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just install-systemd-user
```

The recipe installs `~/.local/bin/qmk-state-daemon`, copies
`systemd/qmk-state-daemon.service` to
`~/.config/systemd/user/qmk-state-daemon.service`, writes a default config
if one does not exist, then enables and starts the user service.

2. If the daemon cannot open the keyboard HID interface, install the udev
rule and reconnect the keyboard:

```bash
just install-udev-rule 303a 4044
# unplug/replug the keyboard
just detect
```

3. Verify the service and daemon RPC:

```bash
just systemd-status
qmk-state-daemon ping
qmk-state-daemon qsid get 28
qmk-state-daemon qsid get 29
```

4. Edit the config at:

```text
${XDG_CONFIG_HOME:-~/.config}/qmk-state-daemon/config.toml
```

`reload-config` applies sink, event, and state-file path changes without
restarting:

```bash
qmk-state-daemon reload-config
```

Restart the service after VID, PID, or socket path changes:

```bash
systemctl --user restart qmk-state-daemon.service
```

## i3status-rust Setup

The recommended i3status-rust integration is `custom_dbus`: i3status-rust
owns a D-Bus object, and the daemon's command sink calls `busctl` on each
deduped keyboard state change. This is event-driven and analogous to the
macOS `sketchybar --trigger` flow, without a polling interval or helper
shell script.

1. Add the D-Bus block to i3status-rust config:

```toml
[[block]]
block = "custom_dbus"
path = "/qmk_state"
format = " $text "
```

This creates a user D-Bus object at service `rs.i3status`, path
`/qmk_state`, interface `rs.i3status.custom`. If you run multiple
i3status-rust bars, set `I3RS_DBUS_NAME` for each bar and update the
daemon command service name accordingly, for example `rs.i3status.top`.

2. Configure the daemon command sink with inline `busctl` commands:

```toml
vid = "0x303A"
pid = "0x4044"

[sink]
kind = "command"
event = "qmk_state_changed"
commands = [
  ["busctl", "--user", "call", "rs.i3status", "/qmk_state", "rs.i3status.custom", "SetIcon", "s", "keyboard"],
  ["busctl", "--user", "call", "rs.i3status", "/qmk_state", "rs.i3status.custom", "SetText", "ss", "{top_layer_name} {mods_letters}", "{top_layer_name}"],
]
```

The template args are expanded by `CommandSink`, so no shell is required.
`SetText` receives full text and short text. The example shows multiple
commands sharing the same event environment; remove the `SetIcon` command
if repeatedly setting a static icon is unnecessary.

3. Reload daemon config:

```bash
qmk-state-daemon reload-config
```

4. Reload/restart the Sway bar so i3status-rust creates the D-Bus object.

5. Verify updates:

```bash
busctl --user introspect rs.i3status /qmk_state rs.i3status.custom
qmk-state-daemon ping
```

Hold a layer key or modifier. The daemon should call `busctl SetText` on
the i3status-rust object immediately after each deduped state change.

### i3status-rust watch_files Fallback

If you do not want to use D-Bus, keep the file-based custom block. The
daemon command sink runs `examples/i3status-rust/qmk-state-to-file.sh` on
each state change, and i3status-rust refreshes via `watch_files`.

1. Find the runtime path i3status-rust should watch:

```bash
printf '%s\n' "${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.i3status"
```

2. Configure the daemon command sink:

```toml
[sink]
kind = "command"
event = "qmk_state_changed"
program = ["/home/alex/bench/cfg/vial-qmk/tools/qmk-state-daemon/examples/i3status-rust/qmk-state-to-file.sh"]
```

3. Add a custom block to i3status-rust. Replace `1000` with the output of
`id -u` if needed:

```toml
[[block]]
block = "custom"
command = "cat /run/user/1000/qmk-state-daemon.i3status 2>/dev/null || printf '?'"
watch_files = ["/run/user/1000/qmk-state-daemon.i3status"]
interval = "once"
format = " $text "
```

4. Verify the file updates:

```bash
cat /run/user/$(id -u)/qmk-state-daemon.i3status
qmk-state-daemon ping
```

The helper writes the watched file in place instead of replacing it with
`mv`, so `watch_files` observes modifications on the configured path.

### Future Native DbusSink

A native `DbusSink` would be an optimization, not a requirement. It would
not create the i3status-rust object; i3status-rust still creates that from
its `custom_dbus` block config. The daemon sink would act as a D-Bus
client and call `SetText` directly from Rust, cutting out the per-event
`busctl` process. Keep the current `CommandSink` approach unless process
spawn overhead becomes measurable or the D-Bus formatting/config needs to
be fully daemon-native.

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
- stale sketchybar or broken `qsid` commands → follow `RUNBOOK.md`
  (especially the `just install` + `launchctl kickstart -k ...` recovery flow).
- need to temporarily use Vial GUI or the Python fallback → use
  `just pause-launchd` / `just resume-launchd` (or `daemon-pause` /
  `daemon-resume` from the keymap tools dir).
- Linux hidraw permission denied → run `just install-udev-rule 303a 4044`,
  unplug/replug the keyboard, then run `just detect` again.
