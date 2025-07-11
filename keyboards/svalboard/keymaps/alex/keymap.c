/*
Copyright 2023 Morgan Venable @_claussen

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 2 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <http://www.gnu.org/licenses/>.
*/

#include "../keymap_support.c"
#include "../keymap_support.h"
#include "action_layer.h"
#include "keycodes.h"
#include "modifiers.h"
#include "quantum_keycodes.h"
#include QMK_KEYBOARD_H
#include <stdbool.h>
#include <stdint.h>
#include "svalboard.h"
#include "vial.h"
// start from last custom qk keycode in keymap_support.h
// SV_SAFE_RANGE is for safe keys only in qmk
// keys we want to show in vial should be QK_KB_0 onwards
#define RANGE_START SV_TOGGLE_AUTOMOUSE + 1

enum custom_keycodes { SPACE_HYPR_L5 = RANGE_START };

// Double hold functionality
typedef struct {
    uint16_t last_hold_time;
    uint16_t current_press_time;
    bool     double_hold_active;
    bool     was_actually_held; // Track if this was a real hold vs quick press
} double_hold_state_t;

#define DOUBLE_HOLD_TIMEOUT 500 // milliseconds
#define MIN_HOLD_DURATION 50    // milliseconds - much lower to catch permissive hold triggers

// Generic function to handle double hold behavior
// Returns true if QMK should continue with default processing, false if handled
bool process_handle_key_actions(uint16_t keycode, keyrecord_t* record, double_hold_state_t* state, uint16_t tap_keycode, uint8_t layer, uint8_t mod, uint16_t timeout) {
    // If this is any kind of tap event, let QMK handle it completely
    // This preserves rapid tapping, tap+hold, and all mod-tap settings
    if (record->tap.count > 0) {
        // Clear any double hold tracking on taps to prevent interference
        state->last_hold_time = 0;
        state->current_press_time = 0;
        state->was_actually_held  = false;
        return true; // Let QMK handle all tap behavior naturally
    }

    // Only handle pure hold events for double hold functionality
    if (record->event.pressed) {
        uint16_t current_time = timer_read();
        state->current_press_time = current_time;
        state->was_actually_held  = false;

        // Check if this is a double hold (hold within timeout of previous hold)
        // Add timer wraparound protection
        if (state->last_hold_time != 0) {
            uint16_t time_diff = current_time - state->last_hold_time;
            // Handle timer wraparound (16-bit timer)
            if (current_time < state->last_hold_time) {
                time_diff = (0xFFFF - state->last_hold_time) + current_time;
            }

            if (time_diff < timeout) {
                // Double hold detected - activate mod + layer
                register_mods(mod);
                layer_on(layer);
                state->double_hold_active = true;
                return false; // Skip default handling
            }
        }

        // Single hold - continue with default mod-tap behavior
        state->double_hold_active = false;
        return true;
    } else {
        // Key released
        if (state->double_hold_active) {
            // Clean up double hold state
            layer_off(layer);
            unregister_mods(mod);
            state->double_hold_active = false;
            state->last_hold_time     = 0;
            state->current_press_time = 0;
            state->was_actually_held  = false;
            return false; // Skip default handling
        } else {
            // Only record timestamp if held long enough to be a legitimate hold
            uint16_t current_time  = timer_read();
            uint16_t hold_duration = current_time - state->current_press_time;

            // Handle timer wraparound
            if (current_time < state->current_press_time) {
                hold_duration = (0xFFFF - state->current_press_time) + current_time;
            }

            if (hold_duration >= MIN_HOLD_DURATION) {
                state->last_hold_time    = state->current_press_time;
                state->was_actually_held = true;
            } else {
                // Clear state completely for quick presses
                state->last_hold_time    = 0;
                state->was_actually_held = false;
            }
            state->current_press_time = 0;
            return true; // Continue with default behavior
        }
    }
}

// Define the list of keys with their configuration
// Format: X(unique_id, keycode_expression, tap_key, layer, mod)
#define DOUBLE_HOLD_KEYS                                                                        \
    X(hypr_bslsh, ALL_T(KC_BACKSLASH), KC_BACKSLASH, 4, MOD_HYPR)                               \
    X(csg_p, MT(MOD_LCTL | MOD_LSFT | MOD_LGUI, KC_P), KC_P, 4, MOD_LCTL | MOD_LSFT | MOD_LGUI) \
    X(csg_o, MT(MOD_LCTL | MOD_LALT | MOD_LGUI, KC_O), KC_O, 4, MOD_LCTL | MOD_LALT | MOD_LGUI) \
    X(asg_i, MT(MOD_LALT | MOD_LSFT | MOD_LGUI, KC_I), KC_I, 4, MOD_LALT | MOD_LSFT | MOD_LGUI) \
    X(meh_u, MT(MOD_MEH, KC_U), KC_U, 4, MOD_MEH)                                               \
    X(cg_scln, MT(MOD_LCTL | MOD_LGUI, KC_SEMICOLON), KC_SEMICOLON, 4, MOD_LCTL | MOD_LGUI)     \
    X(ag_l, MT(MOD_LALT | MOD_LGUI, KC_L), KC_L, 4, MOD_LALT | MOD_LGUI)                        \
    X(gs_k, MT(MOD_LSFT | MOD_LGUI, KC_K), KC_K, 4, MOD_LSFT | MOD_LGUI)                        \
    X(cs_j, MT(MOD_LSFT | MOD_LCTL, KC_J), KC_J, 4, MOD_LSFT | MOD_LCTL)                        \
    X(ca_h, MT(MOD_LCTL | MOD_LALT, KC_H), KC_H, 4, MOD_LCTL | MOD_LALT)                        \
    X(ctl_slash, LCTL_T(KC_SLASH), KC_SLASH, 4, MOD_LCTL)                                       \
    X(alt_comma, LALT_T(KC_COMMA), KC_COMMA, 4, MOD_LALT)                                       \
    X(gui_period, LGUI_T(KC_DOT), KC_DOT, 4, MOD_LGUI)                                          \
    X(csg_q, MT(MOD_LCTL | MOD_LSFT | MOD_LGUI, KC_Q), KC_Q, 6, MOD_LCTL | MOD_LSFT | MOD_LGUI) \
    X(csg_w, MT(MOD_LCTL | MOD_LALT | MOD_LGUI, KC_W), KC_W, 6, MOD_LCTL | MOD_LALT | MOD_LGUI) \
    X(asg_e, MT(MOD_LALT | MOD_LSFT | MOD_LGUI, KC_E), KC_E, 6, MOD_LALT | MOD_LSFT | MOD_LGUI) \
    X(meh_r, MT(MOD_MEH, KC_R), KC_R, 6, MOD_MEH)                                               \
    X(cg_a, MT(MOD_LCTL | MOD_LGUI, KC_A), KC_A, 6, MOD_LCTL | MOD_LGUI)                        \
    X(ag_s, MT(MOD_LALT | MOD_LGUI, KC_S), KC_S, 6, MOD_LALT | MOD_LGUI)                        \
    X(gs_d, MT(MOD_LSFT | MOD_LGUI, KC_D), KC_D, 6, MOD_LSFT | MOD_LGUI)                        \
    X(cs_f, MT(MOD_LSFT | MOD_LCTL, KC_F), KC_F, 6, MOD_LSFT | MOD_LCTL)                        \
    X(ca_g, MT(MOD_LCTL | MOD_LALT, KC_G), KC_G, 6, MOD_LCTL | MOD_LALT)                        \
    X(ctl_z, LCTL_T(KC_Z), KC_Z, 6, MOD_LCTL)                                                   \
    X(alt_c, LALT_T(KC_C), KC_C, 6, MOD_LALT)                                                   \
    X(gui_x, LGUI_T(KC_X), KC_X, 6, MOD_LGUI)

// Generate state variables for each key using the unique identifier
#define X(id, keycode, tap_key, layer, mod) static double_hold_state_t id##_state = {0, 0, false, false};
DOUBLE_HOLD_KEYS
#undef X

bool process_record_user(uint16_t keycode, keyrecord_t* record) {
    switch (keycode) {
        case SPACE_HYPR_L5:
            if (record->event.pressed) {
                // Send the string "hello" when the key is pressed
                send_string("hello");
                return false;
            }

// Generate case statements for all double hold keys
#define X(id, keycode, tap_key, layer, mod) \
    case keycode:                           \
        return process_handle_key_actions(keycode, record, &id##_state, tap_key, layer, mod, DOUBLE_HOLD_TIMEOUT);
            DOUBLE_HOLD_KEYS
#undef X

        default:
            return true;
    }
    return true;
}

#define LAYER_COLOR(name, color) rgblight_segment_t const(name)[] = RGBLIGHT_LAYER_SEGMENTS({0, 2, color})

LAYER_COLOR(layer0_colors, HSV_GREEN);  // NORMAL
LAYER_COLOR(layer1_colors, HSV_GREEN);  // NORMAL_HOLD
LAYER_COLOR(layer2_colors, HSV_ORANGE); // FUNC
LAYER_COLOR(layer3_colors, HSV_ORANGE); // FUNC_HOLD
LAYER_COLOR(layer4_colors, HSV_AZURE);  // NAS
LAYER_COLOR(layer5_colors, HSV_AZURE);  // would be NAS hold
LAYER_COLOR(layer6_colors, HSV_RED);    // maybe 10kp
LAYER_COLOR(layer7_colors, HSV_RED);
LAYER_COLOR(layer8_colors, HSV_PINK);
LAYER_COLOR(layer9_colors, HSV_PURPLE);
LAYER_COLOR(layer10_colors, HSV_CORAL);
LAYER_COLOR(layer11_colors, HSV_SPRINGGREEN);
LAYER_COLOR(layer12_colors, HSV_TEAL);
LAYER_COLOR(layer13_colors, HSV_TURQUOISE);
LAYER_COLOR(layer14_colors, HSV_YELLOW);
LAYER_COLOR(layer15_colors, HSV_MAGENTA); // MBO
#undef LAYER_COLOR

const rgblight_segment_t* const __attribute((weak)) sval_rgb_layers[] = RGBLIGHT_LAYERS_LIST(layer0_colors, layer1_colors, layer2_colors, layer3_colors, layer4_colors, layer5_colors, layer6_colors, layer7_colors, layer8_colors, layer9_colors, layer10_colors, layer11_colors, layer12_colors, layer13_colors, layer14_colors, layer15_colors);

layer_state_t default_layer_state_set_user(layer_state_t state) {
    rgblight_set_layer_state(0, layer_state_cmp(state, 0));
    return state;
}

layer_state_t layer_state_set_user(layer_state_t state) {
    for (int i = 0; i < RGBLIGHT_LAYERS; ++i) {
        rgblight_set_layer_state(i, layer_state_cmp(state, i));
    }
    return state;
}

void keyboard_post_init_user(void) {
    // Customise these values if you need to debug the matrix
    // debug_enable=true;
    // debug_matrix=true;
    // debug_keyboard=true;
    // debug_mouse=true;
    rgblight_layers = sval_rgb_layers;
}

enum layer {
    NORMAL,
    NORMAL_HOLD,
    FUNC,
    FUNC_HOLD,
    NAS,
    MBO = MH_AUTO_BUTTONS_LAYER,
};

// clang-format off
const uint16_t PROGMEM keymaps[DYNAMIC_KEYMAP_LAYER_COUNT][MATRIX_ROWS][MATRIX_COLS] = {
    [NORMAL] = LAYOUT(
        /*Center           North           East            South           West*/

        /*R1*/ KC_J,            KC_U,           KC_QUOTE,       KC_M,           KC_H, XXXXXXX,
        /*R2*/ KC_K,            KC_I,           KC_COLON,       KC_COMMA,       KC_Y, XXXXXXX,
        /*R3*/ KC_L,            KC_O,           KC_LGUI,        KC_DOT,         KC_N, XXXXXXX,
        /*R4*/ KC_SEMICOLON,    KC_P,           KC_BSLS,        KC_SLASH,       KC_RBRC, XXXXXXX,

        /*L1*/ KC_F,            KC_R,           KC_G,           KC_V,           KC_DOUBLE_QUOTE, XXXXXXX,
        /*L2*/ KC_D,            KC_E,           KC_T,           KC_C,           KC_GRAVE, XXXXXXX,
        /*L3*/ KC_S,            KC_W,           KC_B,           KC_X,           KC_ESC, XXXXXXX,
        /*L4*/ KC_A,            KC_Q,           KC_LBRC,        KC_Z,           KC_DEL, XXXXXXX,

        /*Down                  Inner (pad)     Upper (Mode)    O.Upper (nail)  OL (knuckle) Pushthrough*/
        /*RT*/ MO(NAS),         KC_SPACE,       TO(FUNC),       KC_BSPC,        KC_LALT,     TG(NAS),
        /*LT*/ KC_LSFT,         KC_ENTER,       TO(NORMAL),          KC_TAB,         KC_LCTL,     KC_CAPS
        ),

    [NORMAL_HOLD] = LAYOUT(
        /*Center           North           East            South           West*/
        /*R1*/ KC_LEFT,         KC_WH_L,        XXXXXXX,        KC_MS_L,        LCTL(KC_LEFT), XXXXXXX,
        /*R2*/ KC_DOWN,         KC_WH_D,        XXXXXXX,        KC_MS_D,        LCTL(KC_DOWN), XXXXXXX,
        /*R3*/ KC_UP,           KC_WH_U,        XXXXXXX,        KC_MS_U,        LCTL(KC_UP), XXXXXXX,
        /*R4*/ KC_RIGHT,        KC_WH_R,        XXXXXXX,        KC_MS_R,        LCTL(KC_RIGHT), XXXXXXX,

        /*L1*/ XXXXXXX,         XXXXXXX,        XXXXXXX,        KC_BTN1,        XXXXXXX, XXXXXXX,
        /*L2*/ XXXXXXX,         XXXXXXX,        XXXXXXX,        KC_BTN3,        XXXXXXX, XXXXXXX,
        /*L3*/ XXXXXXX,         XXXXXXX,        XXXXXXX,        KC_BTN2,        XXXXXXX, XXXXXXX,
        /*L4*/ DF(NORMAL),      _______,        _______,        XXXXXXX,       _______, XXXXXXX,

        /*Down                  Inner           Upper           Outer Upper     Outer Lower  Pushthrough*/
        /*RT*/ _______,         _______,        _______,        _______,        _______, _______,
        /*LT*/ _______,         _______,        _______,        _______,        _______, _______
        ),

    [FUNC] = LAYOUT(
        /*Center           North           East            South           West*/
        /*R1*/ KC_HOME,         KC_UP,          KC_RIGHT,       KC_DOWN,        KC_LEFT, XXXXXXX,
        /*R2*/ XXXXXXX,         KC_F8,          XXXXXXX,        KC_F7,          KC_END, XXXXXXX,
        /*R3*/ KC_PSCR,         KC_F10,         KC_LGUI,        KC_F9,          KC_INS, XXXXXXX,
        /*R4*/ KC_PAUSE,        KC_PGUP,        KC_F12,         KC_PGDN,        KC_F11, XXXXXXX,

        /*L1*/ KC_HOME,         KC_UP,          KC_RIGHT,       KC_DOWN,        KC_LEFT, XXXXXXX,
        /*L2*/ XXXXXXX,         KC_F6,          XXXXXXX,        KC_F5,          XXXXXXX, XXXXXXX,
        /*L3*/ XXXXXXX,         KC_F4,          XXXXXXX,        KC_F3,          KC_ESC, XXXXXXX,
        /*L4*/ XXXXXXX,         KC_F2,          XXXXXXX,        KC_F1,          KC_DEL, XXXXXXX,

        /*Down                  Inner           Upper           Outer Upper     Outer Lower  Pushthrough*/
        /*RT*/ MO(NAS),         KC_SPACE,       _______,       KC_BSPC,      KC_LALT, _______,
        /*LT*/ KC_LSFT,       KC_ENTER,         _______, KC_TAB,         KC_LCTL, _______
        ),

    [FUNC_HOLD] = LAYOUT(
        /*Center           North           East            South           West*/
        /*R1*/ KC_LEFT,         LCTL(KC_UP),    LCTL(KC_RIGHT), LCTL(KC_DOWN),  LCTL(KC_LEFT), XXXXXXX,
        /*R2*/ KC_UP,           KC_MS_U,        KC_MS_R,        KC_MS_D,        KC_MS_L, XXXXXXX,
        /*R3*/ KC_DOWN,         KC_WH_U,        KC_WH_R,        KC_WH_D,        KC_WH_L, XXXXXXX,
        /*R4*/ KC_RIGHT,        XXXXXXX,        XXXXXXX,        XXXXXXX,        XXXXXXX, XXXXXXX,


        /*L1*/ XXXXXXX,         XXXXXXX,        XXXXXXX,        XXXXXXX,     XXXXXXX, XXXXXXX,
        /*L2*/ XXXXXXX,         XXXXXXX,        XXXXXXX,        XXXXXXX,     XXXXXXX, XXXXXXX,
        /*L3*/ XXXXXXX,         XXXXXXX,        XXXXXXX,        XXXXXXX,     XXXXXXX, XXXXXXX,
        /*L4*/ _______,      _______,        _______,        _______,       _______, XXXXXXX,

        /*Down                  Inner           Upper           Outer Upper     Outer Lower  Pushthrough*/
        /*RT*/ _______,         _______,        _______,        _______,        _______, _______,
        /*LT*/ _______,         _______,        _______,        _______,        _______, _______
        ),

    [NAS] = LAYOUT(
        /*Center           North           East            South           West*/
        /*R1*/ KC_7,            KC_AMPR,        KC_UNDS,        KC_KP_PLUS,     KC_6, XXXXXXX,
        /*R2*/ KC_8,            KC_KP_ASTERISK, KC_COLON,       KC_COMMA,       KC_CIRCUMFLEX, XXXXXXX,
        /*R3*/ KC_9,            KC_LPRN,        KC_LGUI,        KC_DOT,         KC_SEMICOLON, XXXXXXX,
        /*R4*/ KC_0,            KC_RPRN,        XXXXXXX,        KC_QUES,        KC_RBRC, XXXXXXX,

        /*L1*/ KC_4,            KC_DOLLAR,      KC_5,           KC_MINUS,       KC_SLASH, XXXXXXX,
        /*L2*/ KC_3,            KC_HASH,        KC_GT,          KC_PERCENT,     KC_LT, XXXXXXX,
        /*L3*/ KC_2,            KC_AT,          XXXXXXX,        KC_X,           KC_ESC, XXXXXXX,
        /*L4*/ KC_1,            KC_EXCLAIM,     KC_TILDE,       KC_EQUAL,       KC_DEL, XXXXXXX,

        /*Down                  Inner           Upper           Outer Upper     Outer Lower  Pushthrough*/
        /*RT*/ MO(NAS),         KC_SPACE,       _______,       KC_BSPC,        KC_LALT, _______,
        /*LT*/ KC_LSFT,         KC_ENTER,       _______,        KC_TAB,         KC_LCTL, _______
        ),

    [MBO] = LAYOUT(
        /*Center           North           East            South           West*/
        /*R1*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_BTN1,       KC_TRNS, XXXXXXX,
        /*R2*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_BTN3,       KC_TRNS, XXXXXXX,
        /*R3*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_BTN2,       KC_TRNS, XXXXXXX,
        /*R4*/ SV_RECALIBRATE_POINTER,        KC_TRNS,       KC_TRNS,       KC_TRNS,       KC_TRNS, XXXXXXX,
        /*L1*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_BTN1,        KC_TRNS, XXXXXXX,
        /*L2*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_BTN3,        KC_TRNS, XXXXXXX,
        /*L3*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_BTN2,        KC_TRNS, XXXXXXX,
        /*L4*/ SV_RECALIBRATE_POINTER,        KC_TRNS,       KC_TRNS,       KC_TRNS,       KC_TRNS, XXXXXXX,
        /*RT*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_TRNS,       KC_TRNS,   KC_TRNS,
        /*LT*/ KC_TRNS,        KC_TRNS,       KC_TRNS,       KC_TRNS,       KC_TRNS,   KC_TRNS
        )

};
// clang-format on

uint16_t achordion_timeout(uint16_t tap_hold_keycode) {
    // Configure 0 timeout for thumb cluster keys to make them immediately responsive
    switch (tap_hold_keycode) {
        // Thumb cluster keys from the keymap
        case LT(4, KC_BACKSPACE): // leftpad
        case LT(4, KC_ENTER):     // rightpad
            return 0;             // Bypass Achordion timeout for thumb keys
    }

    return 800; // Use 800ms timeout for all other tap-hold keys
}

bool achordion_chord(uint16_t tap_hold_keycode, keyrecord_t* tap_hold_record, uint16_t other_keycode, keyrecord_t* other_record) {
    if (tap_hold_record->event.key.row == 0 || tap_hold_record->event.key.row == 5 || other_record->event.key.row == 0 || other_record->event.key.row == 5) {
        return true;
    }

    return achordion_opposite_hands(tap_hold_record, other_record);
}
