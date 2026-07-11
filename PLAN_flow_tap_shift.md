# Plan: Flow-Tap Shift Tuning + Sketchybar Layer/Mod Indicator

Default branch: `vial`. Working branch base: `feature/initial-customise`.

## Workstream A — Custom Flow-Tap QSIDs for shift mod-taps ✅ COMMITTED

**Goal:** Make `LSFT_T(KC_V)` and `LSFT_T(KC_M)` reliably produce Shift on fast opposite-hand rolls (e.g. `space → M → S` for capital `S`) while preserving Flow Tap streak protection on other mod-taps. Expose the delta and clamp as live-tunable Vial QSIDs.

**Branch:** `feature/initial-customise`, commit `f732d29f42`, pushed to `origin/feature/initial-customise`.

### Status: DONE
- QSID 28 (`flow_tap_shift_delta`, uint16, default 25ms)
- QSID 29 (`flow_tap_shift_min_clamp`, uint8, default 15ms)
- `quantum/qmk_settings.{h,c}` updated; struct size 42, static_assert bumped.
- `get_flow_tap_term` overrides `LSFT_T(KC_V)` and `LSFT_T(KC_M)`: `max(base - delta, clamp)` with uint32 underflow guard.
- CLI: `keyboards/svalboard/keymaps/alex/tools/vial-qs.py` (list/get/set arbitrary QSIDs via hidapi).
- Docs: `keyboards/svalboard/keymaps/alex/README.org` covers QSIDs and Vial GUI concurrency caveat.
- Compile verified: `qmk compile -kb svalboard/trackball/pmw3389/{left,right} -km alex`. UF2s at repo root.
- Awaiting user hardware validation. Do NOT touch this branch during Workstream B.

---

## Workstream B — Sketchybar QMK layer/mod indicator (Rust daemon)

**Goal:** Two sketchybar items updated within 100ms of any layer/mod change on the keyboard. Push-based (firmware sends raw-HID on state change) + pull-based (daemon can query full state on startup / user click). Rust daemon parses, writes JSON, triggers sketchybar. Auto-start via launchd.

### Locked design decisions (per user)

- **Layer names** (short form): `BASE`, `BASE-H`, `FN`, `FN-H`, `NAS`, `NAS-H`, `NUM`, `MBO`, and generic `L7`..`L15` for unused slots.
  - Mapping (matches `enum layer` in `keyboards/svalboard/keymaps/alex/keymap.c:241-248`):
    - 0 `NORMAL` → `BASE`
    - 1 `NORMAL_HOLD` → `BASE-H`
    - 2 `FUNC` → `FN`
    - 3 `FUNC_HOLD` → `FN-H`
    - 4 `NAS` → `NAS`
    - 5 (would-be NAS hold) → `NAS-H`
    - 6 (10kp?) → `NUM`
    - 15 `MBO` → `MBO`
    - other → `L<n>`
- **Layer colors** — source of truth is sketchybar's palette in `~/.config/sketchybar/colors.sh`. Current palette: BLACK, WHITE, RED, GREEN, BLUE, YELLOW, ORANGE, MAGENTA, GREY, TRANSPARENT. If additional distinct colors needed (e.g. CYAN, PURPLE), extend `colors.sh` via chezmoi in the sketchybar-wiring step. Mapping approximates the RGB layer colors in `keymap.c:200-215` but respects the palette:
  - BASE → GREEN
  - BASE-H → GREEN (dimmed via alpha or same)
  - FN → ORANGE
  - FN-H → MAGENTA
  - NAS → BLUE
  - NAS-H → BLUE
  - NUM → RED
  - MBO → MAGENTA
  - fallback → GREY
- **Mod letters** — kanata/QMK convention: `C` Ctrl, `S` Shift, `A` Alt, `G` Gui. Order `CSAG`. All 15 non-empty permutations: `C`, `S`, `A`, `G`, `CS`, `CA`, `CG`, `SA`, `SG`, `AG`, `CSA`, `CSG`, `CAG`, `SAG`, `CSAG`. Icons deferred.
- **Mod state colors** (two-item indicator, `qmk_mods` item):
  - Held real mods → RED
  - Weak mods → ORANGE
  - OSM pending → YELLOW
  - Locked mods → BLUE
  - When multiple states active simultaneously, precedence: locked > held > osm > weak. (Rare in practice; single dominant color keeps the item readable.)
- **Query on click**: both items (`qmk_layer` and `qmk_mods`) subscribe to `mouse.clicked` and invoke `qmk-state-daemon query`, which sends a `0xAC` request packet to firmware.
- **Startup**: daemon sends `0xAC` immediately after connect so state is populated even if user changes nothing.

### Branch strategy

1. `git switch vial && git pull` (sync default).
2. `git switch -c feature/qmk-state-daemon` off `vial`.
3. Build daemon + firmware hooks on this branch.
4. Do NOT push, PR, or merge without explicit user confirmation.
5. After user confirms end-to-end, merge to `vial`, then rebase `feature/initial-customise` on `vial`.

### Packet protocol (raw HID, usage_page 0xFF60 usage 0x61)

**`0xAB` — state snapshot (firmware → daemon), 32 bytes:**
```
0: msg_id = 0xAB
1: version = 1
2: reason bitfield:
     0x01 = layer change
     0x02 = default layer change
     0x04 = real/weak mods change
     0x08 = oneshot mods change
     0x10 = initial snapshot / query response
3: top_layer (uint8) - result of get_highest_layer(layer_state)
4: default_layer (uint8) - get_highest_layer(default_layer_state)
5-8: layer_state (uint32 LE bitmask)
9: real_mods (get_mods())
10: weak_mods (get_weak_mods())
11: oneshot_mods (get_oneshot_mods())
12: locked_mods (get_oneshot_locked_mods())
13-31: reserved, zero
```

**`0xAC` — query request (daemon → firmware), 32 bytes:**
```
0: msg_id = 0xAC
1: version = 1
2-31: zero
```
Firmware responds with `0xAB` packet, reason = 0x10.

Debounce: firmware caches last-sent packet bytes; skip send if unchanged (except query responses always send).

---

### Firmware changes (on `feature/qmk-state-daemon`)

7. **`keyboards/svalboard/keymaps/alex/keymap.c` additions**
   - [ ] Include `raw_hid.h`.
   - [ ] Ensure `RAW_ENABLE = yes` in `rules.mk` (svalboard base likely already has it; verify).
   - [ ] Define `QMK_STATE_MSG_ID = 0xAB`, `QMK_QUERY_MSG_ID = 0xAC`.
   - [ ] `static uint8_t last_sent[32]` cache.
   - [ ] `send_qmk_state(uint8_t reason)`:
     - Build 32-byte packet per protocol above.
     - If reason != 0x10 and packet == last_sent, return.
     - `raw_hid_send(buf, 32)`; update cache.
   - [ ] `layer_state_t layer_state_set_user(layer_state_t state)` → call `send_qmk_state(0x01)` with a temporary local copy (state not yet committed; either recompute inside using the passed state, or defer to matrix_scan hook — simplest: temporarily override globals or just re-read after return via a one-shot flag). Preferred: set a `pending_reason` flag and send from `housekeeping_task_user`, which runs after state commit.
   - [ ] `default_layer_state_set_user` → similar via `pending_reason |= 0x02`.
   - [ ] `post_process_record_user` → `pending_reason |= 0x04` if `get_mods()`/`get_weak_mods()` changed since last call; `pending_reason |= 0x08` if oneshot changed.
   - [ ] `housekeeping_task_user` — if `pending_reason != 0`, call `send_qmk_state(pending_reason)` and clear it. Ensures we always send with committed state.
   - [ ] `keyboard_post_init_user` → set `pending_reason = 0x10`.
   - [ ] `raw_hid_receive(uint8_t *data, uint8_t length)`:
     - If `data[0] == 0xAC`: send state with reason 0x10, bypassing cache check.
     - Preserve any existing raw_hid_receive behavior if svalboard defines one (check first).
   - [ ] Preserve DOUBLE_HOLD_KEYS state machine — must not interfere.

8. **Firmware verify**
   - [ ] Rebuild both variants via nix-shell + qmk.
   - [ ] Confirm `RAW_ENABLE = yes` and no size regression that breaks flash.

### Rust daemon (TDD, on `feature/qmk-state-daemon`)

Location: `tools/qmk-state-daemon/` in vial-qmk repo (per user confirmation).

9. **Toolchain — RESOLVED (no shell.nix changes needed)**
   - Rust 1.93.1 pinned at project level via `mise.toml` at repo root.
   - Mise activates via shell rc; `cargo`/`rustc` available both inside and outside `nix-shell` (nix-shell inherits PATH manipulations from mise activation).
   - Verified: `nix-shell --run 'cargo --version && rustc --version'` returns 1.93.1.
   - Flake port evaluated and rejected: non-trivial (would require porting `niv` sources, avr/arm cross toolchain re-verification, `flake.lock` maintenance) for no additional benefit over `mise.toml` + existing `shell.nix`.
   - `Cargo.toml` will set `rust-version = "1.93"`. Rustfmt/clippy included with mise rust install.

10. **Bootstrap**
    - [ ] `cargo new tools/qmk-state-daemon --name qmk-state-daemon --lib`.
    - [ ] Add `src/main.rs` as binary crate alongside lib (or split into workspace with `qmk-state-daemon-lib` + `qmk-state-daemon-bin` — start simple: single crate with both lib and bin).
    - [ ] Cargo.toml deps: `hidapi = "2"`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `anyhow = "1"`, `clap = { version = "4", features = ["derive"] }`, `thiserror = "1"`, `tempfile = "3"` (dev-dep).
    - [ ] `.gitignore`: `target/`.
    - [ ] Rust edition 2021.

11. **Architecture** (extensible for future HID codes)
    - `src/lib.rs` — re-exports.
    - `src/packet.rs` — `PacketHandler` trait + `Registry`:
      ```rust
      pub trait PacketHandler: Send + Sync {
          fn msg_id(&self) -> u8;
          fn decode(&self, buf: &[u8]) -> anyhow::Result<serde_json::Value>;
      }
      pub struct Registry { handlers: HashMap<u8, Box<dyn PacketHandler>> }
      impl Registry {
          pub fn builder() -> RegistryBuilder { ... }
          pub fn handle(&self, buf: &[u8]) -> anyhow::Result<Option<serde_json::Value>> { ... }
      }
      ```
    - `src/handlers/state.rs` — implements `PacketHandler` for `0xAB`:
      - Decodes to `StatePayload { version, reason, top_layer, top_layer_name, default_layer, layer_state, real_mods, weak_mods, oneshot_mods, locked_mods, mods_letters, mods_state }`.
      - `mods_letters`: computed from active mod bitmask, `CSAG` order.
      - `mods_state`: `"held" | "weak" | "osm" | "locked" | "none"` — precedence locked > held > osm > weak.
      - `top_layer_name`: mapped via `layer_name(index) -> &'static str` in `src/layer_names.rs`.
    - `src/layer_names.rs` — hardcoded array matching svalboard alex keymap enum. Comment cross-referencing `keymap.c:241`.
    - `src/handlers/mod.rs` — module aggregator; future handlers added here.
    - `src/transport.rs` — `trait HidTransport { fn read(&mut self, buf: &mut [u8]) -> Result<usize>; fn write(&mut self, buf: &[u8]) -> Result<usize>; }`. `HidApiTransport` (prod, wraps `hidapi::HidDevice`) + `MockTransport` (Vec of canned responses).
    - `src/sketchybar.rs` — `trait Notifier { fn notify(&self, event: &str) -> Result<()>; }`. `SketchybarNotifier` (exec `sketchybar --trigger <event>`) + `NullNotifier`.
    - `src/output.rs` — `write_state_json(path, value)` atomic write via `NamedTempFile::persist`.
    - `src/main.rs` — CLI parsing, wires transport + registry + output + notifier.

12. **Red-Green-Refactor iterations**
    - **R1: State packet decode**
      - [ ] Red: `packet::tests::decodes_state_from_fixture` — hand-crafted 32-byte buffer with `top_layer=4` (NAS), `real_mods = MOD_BIT(KC_LSFT) | MOD_BIT(KC_LCTL)`. Expect JSON with `top_layer_name = "NAS"`, `mods_letters = "CS"`, `mods_state = "held"`.
      - [ ] Green: implement `StateHandler::decode`.
      - [ ] Refactor: extract `mods_to_letters(bits: u8) -> String` and `mods_precedence(...)`.
    - **R2: Registry dispatch**
      - [ ] Red: two mock handlers with ids `0xAB` and `0xAC`; ensure correct one is called by first byte.
      - [ ] Green: `Registry::handle`.
      - [ ] Refactor: builder API.
    - **R3: JSON atomic writer**
      - [ ] Red: `output::tests::writes_and_replaces_atomically` — using tempdir, verify no partial file visible mid-write; verify overwrite works.
      - [ ] Green: tempfile + persist.
    - **R4: End-to-end with mocks**
      - [ ] Red: `main::tests::e2e_with_mock_transport` — mock transport yields fixture packet, expect JSON file with expected payload + notifier called with `qmk_state_changed`.
      - [ ] Green: wire `run(transport, registry, output, notifier)` loop in `lib.rs`.
    - **R5: Reconnect on disconnect**
      - [ ] Red: mock returns `HidError` twice, then a good packet; expect daemon retries with backoff and recovers.
      - [ ] Green: retry loop with `std::thread::sleep` backoff (100ms → 1s cap).
    - **R6: Query request**
      - [ ] Red: calling `daemon.query()` writes a `0xAC` packet through `transport.write`.
      - [ ] Green: implement `query()`.
    - **R7: Second handler stub (extensibility proof)**
      - [ ] Red: add dummy `0xAD` handler, verify dispatch works alongside `0xAB` without changing registry code.

13. **CLI** (`clap` derive)
    - [ ] `qmk-state-daemon run [--vid <hex>] [--pid <hex>] [--state-file /tmp/qmk_state.json] [--sketchybar-event qmk_state_changed] [--dry-run]`.
    - [ ] `qmk-state-daemon query` — opens device, sends `0xAC`, exits. Sketchybar click handler invokes this.
    - [ ] `qmk-state-daemon once` — read one packet then exit (debug).
    - [ ] VID/PID default to svalboard values from `keyboards/svalboard/*/info.json` (look up during implementation).

14. **Build & install**
    - [ ] `cargo build --release` inside nix-shell.
    - [ ] `justfile` with `build`, `test`, `lint` (clippy), `fmt`, `install`, `install-launchd`, `uninstall-launchd`.
    - [ ] `just install` → copies binary to `~/.local/bin/qmk-state-daemon`.

15. **Sketchybar wiring** (revised: lua, not bash)
    - Live config: `~/.config/sketchybar/lua/`. Existing config uses
      SbarLua (see `~/.config/sketchybar/lua/init.lua` and
      `~/.config/sketchybar/lua/items/`).
    - Sketchybar item registration goes in
      `~/.config/sketchybar/lua/items/qmk-layer.lua` and
      `~/.config/sketchybar/lua/items/qmk-mods.lua`, following the
      same pattern as `items/keyboard-layer.lua` (which was a stub
      for exactly this purpose — now obsolete, can be removed later).
    - Registration in `items/init.lua` adds both to the
      `right_section` bracket next to `battery`.
    - Colors sourced from `~/.config/sketchybar/lua/colors.lua`
      (returned as a Lua table). Palette already covers everything
      needed (green, orange, magenta, blue, red, grey).
    - **Event args, not JSON parsing**: daemon uses
      `sketchybar --trigger qmk_state_changed top_layer_name=… mods_letters=… mods_state=… default_layer_name=…`
      so Lua items read `event.top_layer_name` etc. directly. No jq /
      JSON library needed. JSON file at `/tmp/qmk_state.json` is still
      written for debug / manual inspection.
    - **No click-script wired** — click doesn't need to query since
      daemon keeps state fresh via push + 5s heartbeat. If ever
      needed, add `click_script = "sketchybar --trigger qmk_state_changed …"`
      but simpler to just wait for the next heartbeat.
    - After user confirms end-to-end, persist the two new lua files
      into chezmoi via the `chezmoi` skill (`chezmoi add
      ~/.config/sketchybar/lua/items/qmk-*.lua`) and re-add the
      modified `items/init.lua`. Do NOT persist before green-light.

    Design rationale for lua over bash plugin:
    - User's sketchybar config is fully lua-based via SbarLua.
    - Bash plugin approach required jq + shell interpolation on
      every state change; lua reads event args from sketchybar
      natively.
    - No new plugin script needed — daemon → sketchybar event → lua
      handler is one hop.

16. **launchd auto-start**
    - [ ] `tools/qmk-state-daemon/launchd/com.user.qmk-state-daemon.plist` template with `%HOME%` placeholder.
    - [ ] `KeepAlive = true`, `RunAtLoad = true`, logs → `~/Library/Logs/qmk-state-daemon.{out,err}.log`.
    - [ ] `just install-launchd` copies plist, substitutes `%HOME%`, `launchctl bootstrap gui/$UID <plist>`.
    - [ ] `just uninstall-launchd` — `launchctl bootout` + rm.

17. **Docs** (`tools/qmk-state-daemon/README.md`)
    - [ ] Overview + architecture diagram.
    - [ ] Firmware requirements (raw_hid enabled, hook code — link to `keymap.c` section).
    - [ ] Build instructions (nix-shell + cargo).
    - [ ] Install / uninstall (launchd).
    - [ ] Adding new handlers (extensibility guide — register new `PacketHandler` in `Registry::builder`).
    - [ ] Debugging (`--dry-run`, `once`, log paths, `qmk-state-daemon query`).

### Git dance

18. **Sequence**
    - [ ] `git switch vial && git pull`.
    - [ ] `git switch -c feature/qmk-state-daemon`.
    - [ ] Land shell.nix changes → commit.
    - [ ] Land Rust daemon (TDD) → commit(s) per iteration or squashed.
    - [ ] Land firmware hooks → commit.
    - [ ] Local end-to-end test on hardware.
    - [ ] User confirms → merge to `vial` (prefer `gh pr create --base vial`, wait for user).
    - [ ] After merge: `git switch feature/initial-customise && git rebase vial`.

### Do NOT

- Push, PR, or merge without explicit user confirmation.
- Touch `feature/initial-customise` during Workstream B.
- Disable Vial security features.
- Persist sketchybar config to chezmoi before user confirms end-to-end works.

---

## Subagent fan-out (start-of-Workstream-B)

**Task S1 — shell.nix Rust + zsh integration (parallel, BLOCKS daemon work)**
- Agent: `general` (or specialized nix agent if available).
- Prompt: extend `shell.nix` at repo root with rust stable toolchain (`rustc`, `cargo`, `rustfmt`, `clippy`) plus hidapi macOS build deps. Make the shell use user's zsh with rcfile sourcing so aliases and prompt work. Ensure TMPDIR / HOME remain writable. If mkShell is too limiting, migrate to `flake.nix` + `nix develop` with `rust-overlay` or `fenix` for pinned toolchain. Report rust version pin choice for user confirmation. Verify with `nix-shell --run 'cargo --version && rustc --version'` and by launching interactive shell to confirm zsh + user config. Do NOT commit; leave changes uncommitted for user review.
- Output expected: modified `shell.nix` (and optionally `flake.nix`/`flake.lock`), verification transcript, proposed rust version.

**Handoff gate**: user reviews S1 output → confirms rust version → then daemon TDD proceeds.

---

## Progress log

- 2026-07-10: Workstream A committed on `feature/initial-customise` (`f732d29f42`) and pushed. Awaiting hardware validation.
- 2026-07-10: Workstream B design finalized. Rust 1.93.1 via mise (no shell.nix work needed). Starting `feature/qmk-state-daemon`.
- 2026-07-12: Workstream B firmware landed on `feature/initial-customise` (`72611a00b2`). Push-only design (dropped `0xAC` query due to non-weak `raw_hid_receive` in via.c). 5s heartbeat added.
- 2026-07-12: Daemon dedup landed (`775e393132`); 6 dedup tests cover unit key extraction, single & multiple heartbeat dedup, resume-firing-after-change, and mods-only change detection.
- 2026-07-12: Sketchybar wiring switched to lua (SbarLua) — daemon sends event args, lua items read `event.key` directly. New items `~/.config/sketchybar/lua/items/qmk-{layer,mods}.lua`, registered in `items/init.lua`. Bash plugin/item files removed from daemon repo.
