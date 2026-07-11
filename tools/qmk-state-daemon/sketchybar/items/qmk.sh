#!/usr/bin/env bash
# Sketchybar item bootstrap: adds qmk_layer + qmk_mods items on the
# right side, subscribed to qmk_state_changed and mouse.clicked.
#
# Source this from your sketchybarrc (or from an items/*.sh loader).

qmk_common=(
  update_freq=0
  script="$PLUGIN_DIR/qmk_state.sh"
  click_script="$PLUGIN_DIR/qmk_state.sh"
  padding_left=6
  padding_right=6
  label.padding_left=6
  label.padding_right=6
  background.corner_radius=6
  background.height=22
)

sketchybar --add event qmk_state_changed

sketchybar --add item qmk_layer right \
           --set qmk_layer "${qmk_common[@]}" \
           --subscribe qmk_layer qmk_state_changed mouse.clicked

sketchybar --add item qmk_mods right \
           --set qmk_mods "${qmk_common[@]}" \
           --subscribe qmk_mods qmk_state_changed mouse.clicked

# Prime once so items show something even before the first push.
sketchybar --trigger qmk_state_changed
