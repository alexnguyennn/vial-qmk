#!/usr/bin/env markdown

# qmk-state-daemon Runbook

Recovery steps for the common failure modes seen during local development and daily use.

## Symptoms

- Sketchybar `qmk-layer` / `qmk-mods` items stop changing.
- `qmk-state-daemon ping` still works, but state looks stale.
- `qmk-state-daemon qsid list` returns no QSIDs.
- `qmk-state-daemon qsid get 28` or `get 29` returns `0` unexpectedly.
- `/tmp/qmk_state.json` exists but its mtime stops moving.

## Quick Triage

Run these first:

```bash
qmk-state-daemon ping
launchctl print gui/$UID/com.user.qmk-state-daemon | grep -E "state = |last exit|runs = |pid = "
ls -la /tmp/qmk_state.json /tmp/qmk-state-daemon.sock
tail -20 ~/Library/Logs/qmk-state-daemon.err.log
```

Healthy signals:

- `ping` returns `{ "ok": true, "pong": true }`
- launchd reports `state = running`
- `/tmp/qmk-state-daemon.sock` exists
- `/tmp/qmk_state.json` mtime is recent
- stderr log is not growing with fresh errors

## Known Failure Modes

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

## Full Recovery Sequence

Use this when things look weird and you want a deterministic reset:

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

If that still fails, inspect:

```bash
just launchd-status
just launchd-logs
qmk-state-daemon list
ls -la /tmp/qmk_state.json /tmp/qmk-state-daemon.sock
```

## Sanity Checks

Healthy outputs should include:

- `qmk-state-daemon list` shows `303A:4044  FF60/0061  vial:...  lightly`
- `qmk-state-daemon qsid list` includes:
  - `28  flow_tap_shift_delta          2B`
  - `29  flow_tap_shift_min_clamp      1B`
- `cat /tmp/qmk_state.json` shows current `top_layer_name`
- `sketchybar --query qmk-layer` shows `label.value` set to a real layer like `BASE`, `FN`, `NAS`, `MBO`
