# Plan: Flow-Tap Shift Tuning + Sketchybar Layer/Mod Indicator

Default branch: `vial`. Working branch base: `feature/initial-customise`.

## Workstream A — Custom Flow-Tap QSIDs for shift mod-taps

**Goal:** Make `LSFT_T(KC_V)` and `LSFT_T(KC_M)` reliably produce Shift on fast opposite-hand rolls (e.g. `space → M → S` for capital `S`) while preserving Flow Tap streak protection on other mod-taps. Expose the delta and clamp as live-tunable Vial QSIDs.

**Branch:** continue on `feature/initial-customise` (already has partial edit to `quantum/qmk_settings.h`).

### Tasks

1. **Firmware — struct field additions** (`quantum/qmk_settings.h`)
   - [x] Rename `uint8_t unused;` → `uint8_t flow_tap_shift_min_clamp;` (already done).
   - [x] Append `uint16_t flow_tap_shift_delta;` after `flow_tap_term` (already done).
   - [x] Bump `_Static_assert` to 42 bytes (already done).

2. **Firmware — settings registration** (`quantum/qmk_settings.c`)
   - [ ] Add `DECLARE_STATIC_SETTING(28, flow_tap_shift_delta)` after QSID 27.
   - [ ] Add `DECLARE_STATIC_SETTING(29, flow_tap_shift_min_clamp)`.
   - [ ] Set defaults in `qmk_settings_reset`: `delta = 25`, `min_clamp = 15`.
   - [ ] Rewrite `get_flow_tap_term` for `LSFT_T(KC_V)` / `LSFT_T(KC_M)` to return `max(base - delta, clamp)` with underflow guard.

3. **CLI script** (`keyboards/svalboard/keymaps/alex/tools/vial-qs.py`)
   - [ ] ~50 lines, Python + `hid` pkg.
   - [ ] Subcommands: `list`, `get <qsid>`, `set <qsid> <value>`.
   - [ ] Support 1/2/4-byte widths.
   - [ ] Device discovery via serial-number magic `vial:` prefix + usage page 0xFF60.
   - [ ] Warn if Vial GUI likely running (best-effort ps check, optional).

4. **Verification**
   - [ ] `qmk compile -kb svalboard -km alex` — must succeed with new static-assert.
   - [ ] Flash instructions in README (user does the flash).

5. **Docs**
   - [ ] Update `keyboards/svalboard/keymaps/alex/README.org` with:
     - What QSIDs 28/29 do.
     - How to run `tools/vial-qs.py`.
     - Concurrency caveat with Vial GUI (close GUI before scripting).

6. **Commit and PR**
   - [ ] Single commit on `feature/initial-customise`.
   - [ ] Do NOT push/PR until user confirms.

---

## Workstream B — Sketchybar QMK layer/mod indicator (Rust daemon)

**Goal:** Live sketchybar item showing current top layer + held mods, updated within 100ms of any layer/mod change on the keyboard. Push-based (firmware sends raw-HID packets on state change); Rust daemon receives, writes JSON, triggers sketchybar. Auto-start via launchd.

**Branch strategy:**
1. Create fresh branch `feature/qmk-state-daemon` off the default branch `vial`.
2. Build the daemon in isolation on that branch (Rust project only, plus firmware raw_hid hook).
3. Merge `feature/qmk-state-daemon` into `vial`.
4. Rebase `feature/initial-customise` on top of the merged `vial`.

### Firmware changes

7. **`keymap.c` additions**
   - [ ] Include `raw_hid.h`.
   - [ ] Define `QMK_STATE_MSG_ID 0xAB`.
   - [ ] `send_qmk_state()` — pack layer bitmask, top layer, default layer, real mods, weak mods, oneshot mods into 32-byte buffer, `raw_hid_send`.
   - [ ] Hook `layer_state_set_user` → call `send_qmk_state`; return state.
   - [ ] Hook `default_layer_state_set_user` similarly.
   - [ ] Hook `post_process_record_user` → call `send_qmk_state` (catches MT-hold, OSM).
   - [ ] Hook `keyboard_post_init_user` → initial snapshot.
   - [ ] Design packet layout with a version byte to allow future expansion.

### Rust daemon (TDD)

Location: new directory at repo root: `tools/qmk-state-daemon/`.

Rationale for TDD in a Rust HID daemon:
- Pure parsing / packet decoding logic is easily unit-testable with fixture bytes.
- HID I/O is behind a trait so we can inject a mock transport in tests.
- Extensibility (future HID msg IDs) driven by adding new decoder functions with a red test first.

8. **Bootstrap** (`cargo new tools/qmk-state-daemon --name qmk-state-daemon`)
   - [ ] Cargo.toml: `hidapi = "2"`, `serde = { features=["derive"] }`, `serde_json`, `anyhow`, `clap = { features = ["derive"] }`.
   - [ ] `.gitignore`: `target/`.
   - [ ] Rust edition 2021, MSRV pinned.

9. **Architecture** (extensible for future HID codes)
   - `src/lib.rs` — library.
   - `src/packet.rs` — packet types + decoder registry:
     ```rust
     pub trait PacketHandler {
         fn msg_id(&self) -> u8;
         fn decode(&self, buf: &[u8]) -> anyhow::Result<serde_json::Value>;
     }
     pub struct Registry { handlers: HashMap<u8, Box<dyn PacketHandler>> }
     ```
   - `src/handlers/state.rs` — implements `PacketHandler` for `0xAB` (layer+mods).
   - `src/handlers/mod.rs` — will grow: `state`, `battery`, `matrix`, etc.
   - `src/transport.rs` — `trait HidTransport { fn read(&mut self, buf: &mut [u8]) -> Result<usize>; }` with `HidApiTransport` (prod) and `MockTransport` (tests).
   - `src/sketchybar.rs` — `trait Notifier { fn notify(&self, event: &str); }` with `SketchybarNotifier` and `NullNotifier` (tests).
   - `src/output.rs` — writes JSON atomically (write to tmp, rename).
   - `src/main.rs` — wires transport → registry → output + notifier in a blocking loop with reconnect.

10. **Red-Green-Refactor iterations**
    - **R1: State packet decode**
      - [ ] Red: `packet::tests::decodes_layer_and_mods_from_fixture` — asserts JSON `{ "type":"state", "top_layer":2, "mods":["LSFT"], ... }` from a hand-crafted 32-byte buffer.
      - [ ] Green: implement `StatePacket::decode`.
      - [ ] Refactor: extract mod-bitmask decoding into a helper.
    - **R2: Registry dispatch**
      - [ ] Red: `registry::tests::dispatches_by_msg_id` — two mock handlers with different ids, ensure correct one is called.
      - [ ] Green: `Registry::handle(&buf)` implementation.
      - [ ] Refactor: ergonomic `Registry::builder()` API.
    - **R3: JSON atomic writer**
      - [ ] Red: `output::tests::writes_and_replaces_atomically` — verify no partial file visible mid-write; use tempdir.
      - [ ] Green: write-to-`.tmp` + `rename`.
      - [ ] Refactor.
    - **R4: End-to-end with mocks**
      - [ ] Red: `main::tests::e2e_with_mock_transport` — mock transport yields fixture packet, expect JSON file + notifier called with `"qmk_state_changed"`.
      - [ ] Green: wire in `main` loop with dependency injection.
      - [ ] Refactor.
    - **R5: Reconnect on disconnect**
      - [ ] Red: mock transport returns `HidError`, then a good packet; expect daemon retries and recovers.
      - [ ] Green: retry loop with backoff.
      - [ ] Refactor.
    - **R6: Second handler stub** (proves extensibility)
      - [ ] Red: add fake handler for msg id `0xAC`, verify dispatch works alongside `0xAB`.
      - [ ] Green: no code change needed if R2 done right — validates extensibility.

11. **CLI** (`clap` derive)
    - [ ] `qmk-state-daemon --vid <hex> --pid <hex> --state-file /tmp/qmk_state.json --sketchybar-event qmk_state_changed`.
    - [ ] All flags have sensible defaults.
    - [ ] `--once` flag: read a single packet then exit (useful for scripting/testing).
    - [ ] `--dry-run`: don't invoke sketchybar, just log.

12. **Build & install**
    - [ ] `cargo build --release`.
    - [ ] Install target: `~/.local/bin/qmk-state-daemon` (or `$CARGO_INSTALL_ROOT/bin`).
    - [ ] Makefile or `justfile` with `just install`, `just test`, `just lint`.

13. **Sketchybar wiring**
    - Live config lives at `~/.config/sketchybar/` (chezmoi-managed dotfile tree).
    - [ ] Author `~/.config/sketchybar/plugins/qmk_state.sh` — reads JSON, formats label, `sketchybar --set $NAME label=...`.
    - [ ] Append to user's `~/.config/sketchybar/sketchybarrc`:
      ```bash
      sketchybar --add event qmk_state_changed
      sketchybar --add item qmk_state right \
                 --set qmk_state script="$CONFIG_DIR/plugins/qmk_state.sh" \
                                 update_freq=0 \
                 --subscribe qmk_state qmk_state_changed
      ```
    - [ ] Also ship the reference plugin + snippet inside the daemon repo at `tools/qmk-state-daemon/sketchybar/` for portability.
    - [ ] After user confirms it works end-to-end, persist `~/.config/sketchybar/plugins/qmk_state.sh` and any `sketchybarrc` changes into chezmoi via the `chezmoi` skill (`chezmoi re-add` / `chezmoi add`). Do NOT persist to chezmoi before the user green-lights.

14. **launchd auto-start**
    - [ ] `launchd/com.user.qmk-state-daemon.plist` (template with `%HOME%` placeholder).
    - [ ] `KeepAlive = true` so it restarts on crash / keyboard replug.
    - [ ] `RunAtLoad = true`.
    - [ ] `StandardOutPath` + `StandardErrorPath` → `~/Library/Logs/qmk-state-daemon.{out,err}.log`.
    - [ ] Install script: `just install-launchd` copies plist to `~/Library/LaunchAgents/`, replaces `%HOME%`, `launchctl bootstrap gui/$UID`.
    - [ ] Uninstall script: `just uninstall-launchd`.

15. **Docs** (`tools/qmk-state-daemon/README.md`)
    - [ ] Overview + architecture diagram.
    - [ ] Firmware requirements (raw_hid enabled, hook code).
    - [ ] Build instructions.
    - [ ] Install / uninstall (launchd).
    - [ ] Adding new handlers (extensibility guide).
    - [ ] Debugging (`--dry-run`, log paths).

### Branching, merge, rebase sequence

16. **Git dance**
    - [ ] Ensure current changes are committed on `feature/initial-customise` (workstream A commit).
    - [ ] `git switch vial && git pull` (sync default).
    - [ ] `git switch -c feature/qmk-state-daemon` off `vial`.
    - [ ] Build daemon + firmware hooks on this branch.
    - [ ] Commit(s) on `feature/qmk-state-daemon`.
    - [ ] Merge into `vial`: prefer PR via `gh pr create --base vial`; if user says merge locally, use `git switch vial && git merge --no-ff feature/qmk-state-daemon`.
    - [ ] After merge to `vial`: `git switch feature/initial-customise && git rebase vial`.
    - [ ] Resolve any conflicts (unlikely — different files).

### Do NOT

- Push, create PRs, or merge without explicit user confirmation at each step.
- Disable Vial security features.
- Modify `feature/initial-customise`'s Workstream A commit during Workstream B (rebase after, don't intermix).

---

## Sequencing

1. Workstream A tasks 1–6 → user validates on hardware.
2. Workstream B tasks 7–16 → user validates sketchybar shows layer/mods.

## Progress log

- Workstream A firmware edits applied to `quantum/qmk_settings.{h,c}` and CLI script written to `keyboards/svalboard/keymaps/alex/tools/vial-qs.py`.
- Compile verified: `qmk compile -kb svalboard/trackball/pmw3389/{left,right} -km alex` both green.
- Keymap README.org updated with QSID 28/29 docs and CLI usage.
- Awaiting user flash + on-hardware validation, then commit and move to Workstream B.
