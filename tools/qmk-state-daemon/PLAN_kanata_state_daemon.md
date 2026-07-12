# Kanata State Daemon Implementation Plan

## Goal

Build a Kanata equivalent of `qmk-state-daemon` for SketchyBar that is
event-driven for normal operation. The first local deliverable consumes native
Kanata TCP events for layer state. The parallel upstream deliverable adds
Kanata TCP state events for active modifiers so the daemon can show Ctrl,
Shift, Alt, Gui, and combinations according to Kanata itself.

Keep polling out of the main path. Use heartbeat queries only for reconnect,
drift detection, and degraded fallback.

## Current Constraints

- Kanata v1.12.0 exposes `LayerChange`, `TapActivated`, and `HoldActivated`
  over its optional TCP server.
- Kanata v1.12.0 does not expose active modifiers, one-shot state, caps-word
  state, active layer stack, or transparent key resolution over TCP.
- Blocking on a persistent TCP socket is event-driven. It is not polling.
- SketchyBar already supports the desired integration shape via
  `sketchybar --trigger EVENT key=value ...`.
- The existing QMK daemon already has reusable concepts: sink abstraction,
  atomic state file writes, dedupe, launchd packaging, and SketchyBar event
  arguments.

## Workstream A: Native TCP Layer Daemon

Create a sibling tool, not a direct extension of `qmk-state-daemon` initially:

```text
tools/kanata-state-daemon/
```

Rationale: the QMK daemon is HID/QSID-specific. Kanata is TCP/event-stream
specific. Keeping them separate avoids mixing transport concerns while allowing
the same sink and SketchyBar event model.

### A1. CLI and Config

Implement a small Rust binary:

```text
kanata-state-daemon run [--config PATH] [--addr 127.0.0.1:7070]
kanata-state-daemon ping
kanata-state-daemon write-default-config [--path PATH] [--force]
kanata-state-daemon config-path
```

Default config path:

```text
${XDG_CONFIG_HOME:-~/.config}/kanata-state-daemon/config.toml
```

Default state file:

```text
${XDG_STATE_HOME:-~/.local/state}/kanata-state-daemon/state.json
```

Default event:

```text
kanata_state_changed
```

Suggested config:

```toml
addr = "127.0.0.1:7070"
state_file = "auto"
heartbeat_secs = 60

[sink]
kind = "sketchybar"
event = "kanata_state_changed"
```

### A2. TCP Protocol Client

Implement a line-delimited JSON TCP client:

- Connect to `addr`.
- Send `Hello` if needed for capability discovery.
- Record advertised capabilities.
- Read lines using blocking IO.
- Parse `LayerChange`, `TapActivated`, `HoldActivated`, `CurrentLayerName`,
  `MessagePush`, and unknown messages.
- Ignore unknown messages without killing the daemon.
- Reconnect with bounded exponential backoff on EOF or network error.

On connect or reconnect:

- Request `CurrentLayerName`.
- Use the first `LayerChange` or `CurrentLayerName` response to seed state.
- Emit a SketchyBar event once current state is known.

Normal operation:

- Update only on event-stream messages.
- Do not periodically request state as the main mechanism.
- Deduplicate before writing state or triggering SketchyBar.

Heartbeat fallback:

- Optional low-frequency `RequestCurrentLayerName`, for example every 60s.
- Intended only to detect missed events or recover from protocol drift.
- If heartbeat returns the same layer, do not emit.

### A3. State Model

Initial payload:

```json
{
  "type": "kanata_state",
  "version": 1,
  "connected": true,
  "top_layer_name": "custom",
  "default_layer_name": "default",
  "mods_letters": "",
  "mods_state": "unknown",
  "caps_word_state": "unknown",
  "last_tap_hold_key": "spc",
  "last_tap_hold_result": "hold",
  "source": "kanata_tcp"
}
```

Fields:

- `connected`: whether the daemon currently has a live Kanata TCP connection.
- `top_layer_name`: current Kanata layer from `LayerChange` or reconnect query.
- `default_layer_name`: static config default initially, normally `default`.
- `mods_letters`: empty until upstream modifier events exist or inference is
  explicitly enabled.
- `mods_state`: `unknown`, `none`, `held`, `oneshot`, `locked`, or `mixed`.
- `caps_word_state`: `unknown`, `inactive`, or `active`.
- `last_tap_hold_key` and `last_tap_hold_result`: useful for debugging v1.12
  `TapActivated` and `HoldActivated`; not sufficient as authoritative mod
  state.

### A4. Event Sink Reuse

Either copy the small sink/output modules from `qmk-state-daemon` or extract a
tiny shared crate later. Prefer copying for the first implementation because it
keeps the diff small and avoids a workspace restructure.

Required sinks:

- `sketchybar`: `sketchybar --trigger EVENT key=value ...`
- `command`: useful for testing and non-macOS bars
- `null`: dry-run and tests

Event args to expose:

```text
top_layer_name
default_layer_name
mods_letters
mods_state
caps_word_state
connected
last_tap_hold_key
last_tap_hold_result
```

### A5. Launchd Packaging

Add:

```text
tools/kanata-state-daemon/launchd/com.user.kanata-state-daemon.plist
tools/kanata-state-daemon/justfile
```

Kanata itself must be started with TCP enabled:

```sh
kanata --cfg ~/.config/kanata/macos.kbd --port 127.0.0.1:7070
```

The state daemon should tolerate Kanata not being ready yet by reconnecting.

### A6. Validation

Manual validation:

- Start Kanata with `--port 127.0.0.1:7070`.
- Run `kanata-state-daemon run --dry-run`.
- Hold `@fn`; expect `top_layer_name=function`.
- Hold `@l1`; expect `top_layer_name=custom`.
- Release; expect `top_layer_name=default`.
- Restart Kanata; daemon reconnects and emits recovered state.
- Confirm no SketchyBar triggers happen while state is unchanged.

Automated tests:

- Parse known TCP message fixtures.
- Deduplicate identical state snapshots.
- Format SketchyBar trigger arguments.
- Reconnect loop unit tests with a fake TCP server if practical.

## Workstream B: Upstream Kanata Modifier State Events

Add native Kanata TCP events for active modifier state. This is required for
accurate modifier display according to Kanata. OS-level observation and local
inference are not authoritative enough for the final goal.

### B1. Protocol Shape

Add a new server message, ideally snapshot-shaped rather than delta-only:

```rust
ModifierState {
    real: u8,
    weak: u8,
    oneshot: u8,
    locked: u8,
}
```

JSON example:

```json
{"ModifierState":{"real":5,"weak":0,"oneshot":0,"locked":0}}
```

Bit mapping should match QMK-style ordering if Kanata already has an internal
modifier bit representation:

```text
Ctrl, Shift, Alt, Gui
```

If Kanata distinguishes left/right modifiers internally and the data is cheap
to expose, prefer preserving that detail:

```rust
ModifierState {
    real: Vec<String>,
    weak: Vec<String>,
    oneshot: Vec<String>,
    locked: Vec<String>,
}
```

JSON example:

```json
{"ModifierState":{"real":["lctl","lsft"],"weak":[],"oneshot":[],"locked":[]}}
```

The daemon can collapse this to display letters:

```text
C, S, A, G
```

Recommendation: use arrays of modifier names if accepted upstream. It is more
self-describing and avoids protocol ambiguity. Add compact bitfields only if
Kanata maintainers prefer low allocation and stable internal representation.

### B2. Event Semantics

Emit `ModifierState` whenever the effective modifier snapshot changes.

The event should fire for:

- Physical modifier key down/up processed by Kanata.
- Home-row mod hold activation and release.
- `multi` actions that press/release modifier combinations.
- Weak/virtual modifiers if Kanata has that concept.
- One-shot modifier arm, consume, cancel, or timeout.
- Locked modifier toggles if supported.

The event should not fire if the snapshot is unchanged.

On new TCP client connection:

- Send the current `ModifierState` snapshot, same as current layer seeding.
- Advertise a capability such as `modifier-state` in `HelloOk.capabilities`.

Add client request for reconnect fallback:

```rust
RequestModifierState {}
```

Response can reuse `ModifierState`.

### B3. Caps Word State

The current Kanata config uses:

```lisp
cw (caps-word 2000)
```

Caps word display is valuable because it is a keyboard state mode, but it is
separate from ordinary modifiers. Do not overload `ModifierState` for it.

Add either:

```rust
CapsWordState { active: bool }
```

or a generic state mode event:

```rust
ModeState { name: String, active: bool }
```

Recommendation: start with `CapsWordState { active: bool }` if Kanata has a
clear internal caps-word flag. Consider generic `ModeState` only if there are
multiple existing mode-like features with the same lifecycle.

Event semantics:

- Emit on caps-word activation.
- Emit on caps-word deactivation, timeout, or cancellation.
- Send current state to new TCP clients.
- Add `RequestCapsWordState` only if request/response symmetry is desired.

### B4. Layer Stack and Transparent Resolution

Do not include these in the first upstream PR unless maintainers ask for a
larger state snapshot. They are useful but not required for the SketchyBar
layer/mod/caps-word indicator.

Possible later protocol additions:

```rust
LayerStack { layers: Vec<String> }
ResolvedKey { key: String, layer: String, action: String }
```

### B5. Upstream Implementation Strategy

Expected Kanata files, based on the v1.12 protocol shape:

- `tcp_protocol/src/lib.rs`: add message enums and capabilities.
- `src/tcp_server.rs`: broadcast new state messages and seed new clients.
- Keyboard processing loop/state modules: detect modifier and caps-word changes.
- Existing tap/hold activation event code: use as a pattern for event plumbing.

Implementation requirements:

- Keep events snapshot-based and deduped.
- Avoid event emission inside tight paths if state did not change.
- Add tests for JSON compatibility.
- Update docs with example TCP messages.
- Preserve backwards compatibility by adding new message variants only; do not
  change existing `LayerChange`, `TapActivated`, or `HoldActivated` shapes.

## Workstream C: SketchyBar Items

Add Kanata-specific SketchyBar items under the user SketchyBar config:

```text
~/.config/sketchybar/lua/items/kanata-layer.lua
~/.config/sketchybar/lua/items/kanata-mods.lua
~/.config/sketchybar/lua/items/kanata-caps-word.lua
```

### C1. Layer Item

Subscribe to:

```lua
local EVENT = "kanata_state_changed"
```

Initial layer colors:

```lua
local LAYER_COLORS = {
  default = colors.green,
  custom = colors.blue,
  sym = colors.red,
  function = colors.orange,
  nomods = colors.grey,
}
```

Show `event.top_layer_name`.

If `event.connected == "false"`, hide or show a dim `KAN?` indicator. Prefer
hiding after the QMK gating work exists so inactive keyboard backends do not
clutter the bar.

### C2. Modifier Item

Subscribe to the same event.

Behavior:

- Hide when `mods_letters == ""` or `mods_state == "unknown"`.
- Show letters in `CSAG` order when authoritative upstream state is available.
- Use color by state:
  - `held`: red
  - `oneshot`: yellow
  - `locked`: blue
  - `mixed`: magenta

Do not show inferred modifier state by default unless explicitly configured in
the daemon, because inferred state can be wrong for tap-hold cancellation,
combos, and `multi` release semantics.

### C3. Caps Word Item

Subscribe to the same event.

Behavior:

- Hide when `caps_word_state` is `unknown` or `inactive`.
- Show `CW` when active.
- Use a distinct color from normal Shift, for example blue or magenta.

## Workstream C2: Linux Bar Compatibility

The Kanata TCP daemon should be cross-platform. SketchyBar-specific behavior
must stay behind the sink layer because SketchyBar is macOS-only.

Linux-compatible daemon behavior:

- Keep TCP client, state model, dedupe, atomic state file, and command/null
  sinks platform-independent.
- Compile the `sketchybar` sink only on macOS, same as `qmk-state-daemon`.
- Provide a default Linux config using the `command` sink rather than
  `sketchybar`.
- Add systemd user-service packaging alongside launchd packaging.

Recommended Linux sinks:

- `command` sink for i3status-rust `custom_dbus`, matching the existing QMK
  daemon Linux pattern.
- `command` sink for file-based bars that support watch files.
- Optional future native `dbus` sink if process-spawning `busctl` becomes a
  measurable problem.

Linux config example:

```toml
addr = "127.0.0.1:7070"
state_file = "auto"
heartbeat_secs = 60

[sink]
kind = "command"
event = "kanata_state_changed"
commands = [
  ["busctl", "--user", "call", "rs.i3status", "/kanata_state", "rs.i3status.custom", "SetText", "ss", "{top_layer_name} {mods_letters}", "{top_layer_name}"],
]
```

Linux service files:

```text
tools/kanata-state-daemon/systemd/kanata-state-daemon.service
```

Linux validation should mirror the macOS flow but replace SketchyBar checks with
the configured sink:

- Start Kanata with `--port 127.0.0.1:7070`.
- Start `kanata-state-daemon` under systemd user service or foreground dry-run.
- Hold `@fn` and `@l1`; verify command sink output or bar update.
- Restart Kanata; verify reconnect and state recovery.

Linux caveats:

- Kanata device permissions are separate from this daemon; handle them in the
  Kanata service/setup, not in `kanata-state-daemon`.
- Do not add macOS modifier observers to the portable daemon path. If an
  approximation layer is later desired on Linux, it needs a Linux-specific input
  observer and should remain optional because it is not authoritative Kanata
  state.
- QMK daemon Linux compatibility already uses command/file/D-Bus style sinks;
  keep Kanata aligned with that instead of inventing a second Linux bar model.

## Workstream D: QMK Item Visibility Gating

Finish off the existing QMK SketchyBar integration so QMK items show only when
the QMK device is connected and `qmk-state-daemon` is running.

### D1. Daemon State Additions

Extend QMK daemon emitted payload/event args with:

```text
connected=true
backend=qmk
```

For normal HID state packets, `connected=true` is implied.

When the daemon starts but cannot open HID, current behavior likely exits. Keep
that behavior for the daemon, but make SketchyBar detect daemon liveness.

### D2. SketchyBar Liveness Check

Add a lightweight periodic or event-triggered script that checks:

```sh
qmk-state-daemon ping
```

and optionally:

```sh
qmk-state-daemon list
```

Rules:

- If `ping` fails, hide `qmk-layer` and `qmk-mods`.
- If `ping` succeeds but no recent state file exists, hide or show dim `QMK?`.
- If a fresh state event arrives, show items normally.

This liveness check can poll at a low rate, for example 30-60s, because it is
not the main state update mechanism. The main QMK state path remains push-only
from firmware events.

### D3. Better Event-Driven QMK Gating Later

Optional improvement:

- On daemon startup, emit `qmk_state_changed connected=true` after the first
  state packet.
- On graceful daemon shutdown, best-effort emit `connected=false`.
- Add a launchd keepalive status item or wrapper on macOS, and an equivalent
  systemd user-service status command or bar command on Linux.

Do not rely only on graceful shutdown events because crashes, unplug events,
and machine sleep can skip cleanup.

### D4. UI Behavior

QMK item rules:

- `qmk-layer`: hidden until first valid state event or successful liveness
  check with fresh state.
- `qmk-mods`: hidden when no mods, hidden when daemon is not live.
- If both QMK and Kanata are active, show both only if desired; otherwise add a
  config option to prefer Kanata when connected.

## Workstream E: Push-Msg Annotations Without Forking Kanata

Kanata `push-msg` can emit custom `MessagePush` values over TCP. This allows
some richer state without a Kanata fork, but it still needs the same TCP daemon
to receive messages and trigger SketchyBar.

### E1. What Works Well

Push messages work well for explicit mode-like state transitions where the
config has clear activation and deactivation points:

- Entering a custom mode.
- Leaving a custom mode.
- Explicit layer key wrappers if `LayerChange` is insufficient.
- Explicit caps-word activation marker.

Example concept:

```lisp
cw (multi (push-msg {"caps_word":"active"}) (caps-word 2000))
```

This can show caps-word activation, but it cannot reliably show deactivation
unless Kanata also supports a hook when caps-word times out/cancels. Without a
deactivation hook, the daemon would need a timeout heuristic, which is not fully
authoritative.

### E2. Modifier State Feasibility

Modifier display via `push-msg` is possible only as approximation unless Kanata
has reliable press and release instrumentation points for every modifier action.

Problems for the current config:

- Home-row mods use `tap-hold-release-keys`; the hold branch is selected after
  timing and same-hand/opposite-hand logic.
- `multi` aliases such as `@cs`, `@meh`, and `@hypr` press combinations, but a
  single push message at activation does not automatically tell the daemon when
  Kanata releases those mods.
- Chords can emit `@hypr` with `all-released`; this is another path that would
  need annotations.
- Tap cancellation and early tap behavior can make naive key-down/key-up
  tracking wrong.
- Caps-word timeout/cancel is internal and not naturally mirrored by config-only
  push messages.

Performance is acceptable if push messages are emitted only on state changes.
The TCP stream and daemon are cheap. The risk is correctness and config
maintenance, not runtime cost.

### E3. Possible Annotation Design

If we choose config-only inference, keep it explicit and opt-in:

```json
{"mods":{"event":"down","mods":["ctrl","shift"]}}
{"mods":{"event":"up","mods":["ctrl","shift"]}}
{"caps_word":{"event":"active","timeout_ms":2000}}
```

Daemon behavior:

- Maintain an annotation-derived modifier set.
- Clear all inferred modifiers on `LayerChange` to `default` only if explicitly
  configured, because layer release does not necessarily mean modifier release.
- Clear inferred modifiers after a short safety timeout if no release arrives.
- Mark `mods_state=inferred` so SketchyBar can style or hide it differently.

This should not be the default display path because it can lie.

### E4. Tradeoff Summary

Native upstream state events:

- Correct according to Kanata.
- Low config maintenance.
- Best long-term path.
- Requires upstream patch/release or local fork.

Push-msg annotations:

- Works on stock Kanata.
- Event-driven and performant.
- Good for explicit custom modes.
- Weak for authoritative modifier and caps-word deactivation state.
- High config maintenance for home-row mods, combos, and multi-mod aliases.

OS-level modifier observer:

- Event-driven on macOS.
- Good for visual approximation.
- Not according to Kanata internals.
- Cannot distinguish one-shot, weak, locked, pending tap-hold, or suppressed
  remap behavior accurately.
- Linux would need a separate evdev/libinput/XKB-style observer with the same
  correctness limitations, so keep this out of the primary cross-platform
  design.

Recommended path:

- Use native TCP for layers immediately.
- Use v1.12 `HoldActivated`/`TapActivated` for debug and future inference only.
- Do not show modifier state by default until upstream `ModifierState` exists.
- Use push-msg only for explicit mode labels or experimental inferred displays.
- Add upstream `ModifierState` and `CapsWordState` events for authoritative UI.

## Suggested Execution Order

1. Implement `kanata-state-daemon` layer-only MVP.
2. Add Kanata SketchyBar layer item.
3. Add QMK SketchyBar visibility gating.
4. Add optional handling of v1.12 `TapActivated` and `HoldActivated` in the
   daemon state payload.
5. Prototype push-msg annotations for caps-word activation only, clearly marked
   as inferred/timeout-based.
6. Prepare upstream Kanata PR for `ModifierState`.
7. Extend upstream PR or follow-up PR with `CapsWordState`.
8. Enable Kanata modifier and caps-word SketchyBar items once authoritative
   events exist.

## Open Questions

- Should Kanata and QMK indicators be mutually exclusive, or can both be shown
  when both backends are active?
- Should inferred push-msg state ever be shown by default, or only in a debug
  item?
- Should upstream Kanata expose modifier state as compact bitfields, modifier
  name arrays, or both?
- Should caps-word be its own event or part of a generic mode-state event?
