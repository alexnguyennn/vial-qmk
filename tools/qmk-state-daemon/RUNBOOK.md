#!/usr/bin/env markdown

# qmk-state-daemon Runbook

Recovery steps for the common failure modes seen during local development and daily use.

## Symptoms

- Sketchybar `qmk-layer` / `qmk-mods` items stop changing.
- `qmk-state-daemon ping` still works, but state looks stale.
- `qmk-state-daemon qsid list` returns no QSIDs.
- `qmk-state-daemon qsid get 28` or `get 29` returns `0` unexpectedly.
- The state JSON exists but its mtime stops moving.

## Quick Triage

Run these first:

```bash
qmk-state-daemon ping
launchctl print gui/$UID/com.user.qmk-state-daemon | grep -E "state = |last exit|runs = |pid = "
ls -la /tmp/qmk_state.json /tmp/qmk-state-daemon.sock
tail -20 ~/Library/Logs/qmk-state-daemon.err.log
```

Linux/systemd equivalent:

```bash
qmk-state-daemon ping
systemctl --user status qmk-state-daemon.service --no-pager
journalctl --user -u qmk-state-daemon.service -n 30 --no-pager
ls -la "${XDG_STATE_HOME:-$HOME/.local/state}/qmk-state-daemon/state.json" "${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.sock"
```

Healthy signals:

- `ping` returns `{ "ok": true, "pong": true }`
- launchd reports `state = running`
- `/tmp/qmk-state-daemon.sock` exists
- state JSON mtime is recent
- stderr log is not growing with fresh errors

## Known Failure Modes

### 0. I want to use Vial GUI or the direct Python tool temporarily

The daemon is the sole raw-HID owner. Vial GUI and
`keyboards/svalboard/keymaps/alex/tools/vial-qs.py` use the same HID
interface, so they **will** fight the daemon.

There is currently no in-process daemon suspend flag; stop the service
manager job.

Pause / resume flow:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just pause-launchd

# now use Vial GUI, or:
cd ~/bench/cfg/vial-qmk/keyboards/svalboard/keymaps/alex/tools
just py-list
just py-get 28
just py-set 28 40

cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just resume-launchd
sketchybar --reload
```

If you prefer to stay in the keymap tools directory:

```bash
cd ~/bench/cfg/vial-qmk/keyboards/svalboard/keymaps/alex/tools
just daemon-pause
# use Vial GUI or py-* recipes
just daemon-resume
```

Notes:

- Use daemon RPC (`qmk-state-daemon qsid ...`) for normal QSID work.
- Use Vial GUI only for Vial-visible settings / keymap changes.
- Vial GUI still cannot see custom QSIDs 28/29.

Linux/systemd flow:

```bash
systemctl --user stop qmk-state-daemon.service
# use Vial GUI or direct HID tooling
systemctl --user start qmk-state-daemon.service
```

### 1. Daemon crashed at startup on VID/PID parsing

Old builds parsed clap's decimal defaults as hex and crashed with:

```text
error: invalid value '12346' for '--vid <VID>': number too large to fit in target type
```

Fix:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just install
launchctl kickstart -k gui/$UID/com.user.qmk-state-daemon
```

### 2. Broker went idle-blocked, QSID RPC stalled, state stopped updating

Old builds used blocking HID reads in the broker loop, so while the board was idle the daemon stopped servicing RPC requests until the next packet arrived.

Symptoms:

- `qmk-state-daemon qsid list` empty
- `qmk-state-daemon qsid get 28` returns `0`
- sketchybar item appears frozen

Fix:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just install
launchctl kickstart -k gui/$UID/com.user.qmk-state-daemon
```

Then verify:

```bash
qmk-state-daemon qsid list | tail
qmk-state-daemon qsid get 28
qmk-state-daemon qsid get 29
```

Expected:

- QSID 28 present as `flow_tap_shift_delta`
- QSID 29 present as `flow_tap_shift_min_clamp`
- values usually `25` / `15` unless tuned otherwise

### 3. Sketchybar item is stale after daemon recovery

The daemon dedupes unchanged state packets, so if sketchybar reloads after the daemon has already emitted its initial event, the bar may show the placeholder until the next real state change.

Fix:

```bash
sketchybar --reload
```

If still stale, trigger a real keyboard state change:

- hold a modifier
- switch to another layer briefly
- wait for the next heartbeat plus a real state change if needed

Then verify:

```bash
sketchybar --query qmk-layer
sketchybar --query qmk-mods
cat /tmp/qmk_state.json
```

### 3b. Linux bar output is stale after config changes

If only the sink command, event name, or state-file path changed, reload
config in-process:

```bash
qmk-state-daemon reload-config
```

If VID, PID, socket path, or hidraw permissions changed, restart the
service:

```bash
systemctl --user restart qmk-state-daemon.service
journalctl --user -u qmk-state-daemon.service -n 30 --no-pager
```

If the daemon cannot open hidraw, install or refresh the udev rule:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just install-udev-rule 303a 4044
# unplug/replug the keyboard
just detect
```

For i3status-rust, verify the custom block uses `watch_files` and
`interval = "once"` with the same concrete path that the helper writes:

```toml
[[block]]
block = "custom"
command = "cat /run/user/1000/qmk-state-daemon.i3status 2>/dev/null || printf '?'"
watch_files = ["/run/user/1000/qmk-state-daemon.i3status"]
interval = "once"
format = " $text "
```

Check the file directly:

```bash
cat "${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.i3status"
```

If the file changes but the bar does not, restart/reload the Sway bar so
i3status-rust reloads its config. If the file does not change, check the
daemon command sink path in
`${XDG_CONFIG_HOME:-$HOME/.config}/qmk-state-daemon/config.toml` and run
`qmk-state-daemon reload-config`.

### 4. Python fallback script cannot open HID

The fallback script `keyboards/svalboard/keymaps/alex/tools/vial-qs.py` is direct-HID and cannot share the interface with the daemon.

If you need the fallback script:

```bash
launchctl bootout gui/$UID/com.user.qmk-state-daemon
cd ~/bench/cfg/vial-qmk/keyboards/svalboard/keymaps/alex/tools
just py-list
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.user.qmk-state-daemon.plist
```

Normally prefer the daemon RPC path:

```bash
cd ~/bench/cfg/vial-qmk/keyboards/svalboard/keymaps/alex/tools
just list
just flow-status
```

On Linux, replace the launchctl commands with:

```bash
systemctl --user stop qmk-state-daemon.service
systemctl --user start qmk-state-daemon.service
```

## Full Recovery Sequence

Use this when things look weird and you want a deterministic reset:

macOS:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just install
launchctl kickstart -k gui/$UID/com.user.qmk-state-daemon
sleep 2
qmk-state-daemon ping
qmk-state-daemon qsid list | tail
qmk-state-daemon qsid get 28
qmk-state-daemon qsid get 29
sketchybar --reload
```

Linux:

```bash
cd ~/bench/cfg/vial-qmk/tools/qmk-state-daemon
just install
systemctl --user restart qmk-state-daemon.service
sleep 2
qmk-state-daemon ping
qmk-state-daemon qsid list | tail
qmk-state-daemon qsid get 28
qmk-state-daemon qsid get 29
```

If that still fails, inspect:

```bash
just launchd-status
just launchd-logs
qmk-state-daemon list
ls -la /tmp/qmk_state.json /tmp/qmk-state-daemon.sock
```

Linux equivalent:

```bash
just systemd-status
just systemd-logs
qmk-state-daemon list
ls -la "${XDG_STATE_HOME:-$HOME/.local/state}/qmk-state-daemon/state.json" "${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.sock"
```

## Sanity Checks

Healthy outputs should include:

- `qmk-state-daemon list` shows `303A:4044  FF60/0061  vial:...  lightly`
- `qmk-state-daemon qsid list` includes:
  - `28  flow_tap_shift_delta          2B`
  - `29  flow_tap_shift_min_clamp      1B`
- `cat /tmp/qmk_state.json` shows current `top_layer_name`
- `cat ${XDG_STATE_HOME:-$HOME/.local/state}/qmk-state-daemon/state.json` shows current `top_layer_name`
- `sketchybar --query qmk-layer` shows `label.value` set to a real layer like `BASE`, `FN`, `NAS`, `MBO`
