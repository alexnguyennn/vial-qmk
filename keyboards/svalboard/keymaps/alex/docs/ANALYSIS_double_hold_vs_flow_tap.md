# Analysis: `DOUBLE_HOLD_KEYS` vs Flow Tap, Permissive Hold, Chordal Hold

Companion to [`TOUR_double_hold.md`](./TOUR_double_hold.md). Answers:

1. Does the double-hold mechanic *add* accidental-mod risk on top of the
   existing tap-hold features?
2. Do the Flow Tap / Chordal Hold / Permissive Hold settings we have (and are
   tuning) protect double-hold from misfiring, or fight it?
3. What term values are safe?
4. Is Chordal Hold actually taking effect for the tmux combo
   (`C_S(KC_MINUS)` fired by `f+d` / `j+k`)? Are the occasional "tmux transposes
   panes/windows unexpectedly" incidents chordal-hold failures or something
   else?

## TL;DR

- **Double-hold does not increase accidental mod-tap risk.** It only reacts
  *after* QMK has already resolved an event as a hold. If Flow Tap / Chordal
  Hold / Permissive Hold force the event to be a *tap*, double-hold cleanly
  short-circuits.
- **Flow Tap tuning helps double-hold**, not hurts it. Force-tapping fast
  presses stops them from accidentally recording as "first hold" of a
  double-hold sequence.
- **The 500 ms `DOUBLE_HOLD_TIMEOUT` and 50 ms `MIN_HOLD_DURATION` are safe
  with any reasonable `FLOW_TAP_TERM` / `TAPPING_TERM`.** Only pathological
  values (`TAPPING_TERM` > 500 or `MIN_HOLD_DURATION` reduced below 30) would
  cause interaction bugs.
- **Chordal Hold does *not* apply to the tmux combo.** The tmux prefix
  `C_S(KC_MINUS)` is emitted by a **Vial combo** (`f + d` → single virtual
  keycode), and Chordal Hold explicitly returns `true` for combos (see
  `quantum/action_tapping.c:757–759`). So combos are exempt from the
  opposite-hand rule.
- **The intermittent "tmux transposes" is *not* a chordal-hold failure.** It's
  either (a) the combo firing unintentionally because `f + d` (or `j + k`) got
  pressed within `COMBO_TERM` — see the combo term and permissive-hold
  interaction below — or (b) an accidental home-row `Ctrl+Shift` mod-tap
  activation followed by `KC_MINUS`. Diagnosis and fixes below.

---

## 1. How double-hold composes with the tap-hold pipeline

QMK's decision order for a mod-tap key press:

```
event arrives
  │
  ├─ Flow Tap: was previous key within FLOW_TAP_TERM?
  │     → if yes and both keys are flow-tap-eligible: force-tap
  │
  ├─ Chordal Hold: does opposite-hand rule apply?
  │     → if same-hand nested: force-tap
  │
  ├─ Permissive Hold / HOLD_ON_OTHER_KEY_PRESS / TAPPING_TERM:
  │     → decide tap vs hold based on timing + nesting
  │
  └─ dispatch to process_record_user with tap.count set
```

`process_record_user` then calls `process_handle_key_actions`. That function
branches purely on `record->tap.count` and `record->event.pressed`:

- `tap.count > 0`: **tap already decided**. Clear double-hold timestamps,
  return `true` → QMK sends the tap. No double-hold interference.
- `tap.count == 0 && pressed`: **hold**. Check for double-hold arming; if
  armed, take over (register mod, layer_on). Else, save timestamp, fall
  through to normal hold.
- `tap.count == 0 && released`: mirror the above.

**Consequence**: double-hold is *downstream* of every tap-hold feature. Making
those features more aggressive (shorter Flow Tap term, stricter Chordal Hold)
just means fewer events reach the "hold" branch of `process_handle_key_actions`,
which is fine — the double-hold state simply doesn't advance and stays reset.

## 2. Does double-hold add accidental-mod risk?

**No.** The relevant cases:

| Scenario | Without double-hold | With double-hold |
|---|---|---|
| Fast type "as" on left home row | Flow Tap force-taps `A` and `S` → letters | Same — `tap.count > 0` short-circuits |
| Deliberate hold `S` for 200 ms then use | Alt+GUI (LAG) fires normally | Same, plus timestamp saved |
| Same key held twice within 500 ms | Two independent Alt+GUI holds | Second becomes Alt+GUI + NAS layer, **intended** |
| Two different mod-taps in a row | Independent | Independent — each key has its own state variable |

The `state->last_hold_time` reset happens **on every tap**, so a stray fast tap
between two intentional holds erases the double-hold arming window. This is
robust.

The one theoretical risk: if you hold `S`, release, then hold `S` again within
500 ms *without meaning to double-hold*, you get Alt+GUI + NAS instead of just
Alt+GUI. In practice this requires deliberately re-holding the same key twice
in ½ second — not a common typing pattern.

## 3. Safe term values

Recommended (matches current setup or the planned Flow-Tap-shift-delta tuning):

| Setting | Recommended | Rationale |
|---|---|---|
| `TAPPING_TERM` | 175–220 ms | Long enough that deliberate holds register, short enough that permissive-hold doesn't wait forever. |
| `FLOW_TAP_TERM` | 100–150 ms | Kanata used 100. 80 is aggressive and starves shift-tap (see the shift-tap plan). |
| `flow_tap_shift_delta` (QSID 28) | 25–75 ms | Shorter effective window for `LSFT_T(V)` / `LSFT_T(M)` so shift can still hold at speed. |
| `flow_tap_shift_min_clamp` (QSID 29) | 15 ms | Never let the shift-tap window collapse below this. |
| `COMBO_TERM` | Vial slider (see `settings.2` in `.vil` = 60) | Tighter combo window reduces accidental combo firing. **Consider raising to 40–50 ms if tmux prefix fires unexpectedly? Actually, tighter is better — see §4.** |
| `DOUBLE_HOLD_TIMEOUT` | 500 ms (default) | Feels natural for a two-beat gesture. |
| `MIN_HOLD_DURATION` | 50 ms (default) | Anything above `~40` filters out inadvertent bumps. |
| `PERMISSIVE_HOLD` | on | Now safe once Flow Tap + Chordal Hold + shift-tap-delta are in place. |
| `HOLD_ON_OTHER_KEY_PRESS` | off | Too aggressive; will misfire on rolls. |

Danger zone:

- `FLOW_TAP_TERM = 0` → disables Flow Tap entirely. Reverts to
  Permissive-Hold-only, which was your original misfire scenario.
- `FLOW_TAP_TERM ≥ TAPPING_TERM` (both ≥ 200) → shift-tap can never hold
  during a typing streak. Only escape is the shift-delta override.
- `MIN_HOLD_DURATION < 30` → risk of a very fast bump registering as "hold" and
  arming double-hold spuriously on the next press.

## 4. The tmux prefix and combos

### How the combo is wired

From `keymap_alex-update.vil`:

```
combo[7]:  ["C_S_T(KC_F)", "SGUI_T(KC_D)", "KC_NO", "KC_NO", "C_S(KC_MINUS)"]
combo[8]:  ["C_S_T(KC_J)", "SGUI_T(KC_K)", "KC_NO", "KC_NO", "C_S(KC_MINUS)"]
```

Meaning: pressing `f + d` (or `j + k`) within `COMBO_TERM` (60 ms in your
`settings.2`) fires `C_S(KC_MINUS)` = Ctrl+Shift+Minus = your tmux prefix.

### Does Chordal Hold protect this combo?

**No — combos are exempt.** From `quantum/action_tapping.c:757–759`:

```c
if (tap_hold_record->event.type != KEY_EVENT ||
    other_record->event.type != KEY_EVENT) {
    return true; // Return true on combos or other non-key events.
}
```

The combo processor sees `f + d` as a single virtual keycode (`C_S(KC_MINUS)`),
not as two tap-hold keys interacting. Chordal Hold's opposite-hand rule never
runs for the combo itself. It only runs on the *individual* `C_S_T(KC_F)` and
`SGUI_T(KC_D)` presses when *not* completing a combo — and those are same-hand
(both left index/middle), so Chordal Hold would correctly force-tap them if
they weren't intercepted by the combo first.

### Sources of unwanted tmux prefix firing

Ranked by likelihood:

1. **Unintended `f + d` or `j + k` combo trigger during typing.** With
   `COMBO_TERM = 60 ms`, any near-simultaneous press of both letters fires the
   combo. Fast-typed English contains `fd`/`df` (unusual) but *rarely* within
   60 ms. However `jk` (like "flanke**jk**a"? unlikely) is also rare. **Most
   likely culprit: `j + k` during vim navigation** if you rest fingers on
   home row — pressing `j` then `k` quickly for line motion can trip the
   combo. Vim's `jk` escape mapping in particular is exactly this pattern.
2. **Home-row mod misfire producing `C-S-<something>` next to `KC_MINUS`.**
   `LSFT_T(KC_M)` + `LCTL_T(KC_Z)` held simultaneously with `KC_MINUS`
   pressed elsewhere. This does *not* produce Ctrl+Shift+Minus specifically
   unless `KC_MINUS` (which lives on the right-outer column plain — see
   `.vil` layout row-index `[3][4]` = `LSFT(KC_MINUS)` for underscore, and
   `KC_MINUS` on layer 4 index 4) is also pressed. Unlikely mechanism.
3. **Double-hold interaction with the combo?** No: double-hold only fires on
   deliberate hold-release-hold. Combo processing is upstream and short-circuits
   before double-hold sees anything.

### Diagnosis to run

To distinguish #1 from #2:

1. **Enable QMK console** temporarily. Add to `rules.mk`:
   ```
   CONSOLE_ENABLE = yes
   ```
   Add to `keymap.c` `process_record_user`:
   ```c
   uprintf("KC=%04x tap=%u pressed=%u t=%u\n",
           keycode, record->tap.count, record->event.pressed,
           record->event.time);
   ```
   Run `qmk console`, reproduce the misfire, observe.
2. **Or**: temporarily *disable* the tmux combo in Vial and watch whether
   tmux still transposes. If it does → not the combo. If it stops → confirmed
   combo misfire.

### Fixes if the combo is the culprit

- **Tighten `COMBO_TERM` to 30–40 ms** in Vial. Very hard to fire accidentally,
  still easy to fire deliberately with a slight two-finger squeeze.
- **Change the combo trigger keys** to less-frequent pairs — e.g., use
  `f + v` (index + bottom-index) which almost never coincide in English text,
  instead of `f + d` (index + middle, a common bigram approach).
- **Add a require-hold guard** (kanata `first-release` semantics) — QMK combos
  support `COMBO_MUST_HOLD_PER_COMBO` if you enable it in `config.h` and add
  a `bool get_combo_must_hold(uint16_t index, combo_t *combo)` callback that
  returns `true` for the tmux combo. Then the combo only fires if both keys
  are still held at the term deadline, not on transient roll.

### Fixes if home-row misfire is the culprit

- Complete the planned Flow-Tap-shift-delta work (QSIDs 28/29) — reduces
  shift-tap accidental firing which is the most common home-row-mod misfire.
- Consider adding `get_permissive_hold()` per-key overrides to relax
  permissive-hold on high-misfire keys.

## 5. Summary answer to your question

> "do they create more accidental firing of mod tap risk or will tuning flow tap term like we're planning also prevent accidental double hold firing?"

**Tuning Flow Tap prevents accidental double-hold firing as a side benefit.**
Because double-hold only advances state on genuine "hold" events, and Flow Tap
force-tapping fast presses means fewer holds are recorded, the double-hold
arming window becomes strictly harder to hit accidentally. Same applies to
Chordal Hold: same-hand nested presses get force-tapped, so their releases
never register as `last_hold_time` for double-hold purposes.

Net: the four features (double-hold, Flow Tap, Chordal Hold, Permissive Hold)
compose cleanly. Double-hold rides on top of everything else and only kicks in
for *deliberate* holds.

> "also confirm if the chordal hold handedness is taking effect, now and then
> i experience accidental tmux layout transposing"

Chordal Hold **is** taking effect for regular home-row mod-tap presses — the
handedness matrix at `keyboards/svalboard/svalboard.c:305–317` correctly
partitions left/right finger clusters with thumbs marked exempt (`'*'`).

But Chordal Hold **does not run for combos** by design. Your tmux prefix
fires from a Vial combo, so Chordal Hold cannot prevent an unwanted combo
trigger. If tmux is transposing at unexpected times, that is (most likely) an
unintended `j + k` or `f + d` co-press within your 60 ms `COMBO_TERM`, not a
Chordal Hold failure. Run the console diagnosis in §4 to confirm, then
tighten `COMBO_TERM` or switch to a `must-hold` combo variant.
