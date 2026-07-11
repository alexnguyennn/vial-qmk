#!/usr/bin/env bash
# Plugin: reads /tmp/qmk_state.json (written by qmk-state-daemon) and
# updates the qmk_layer + qmk_mods sketchybar items.
#
# Subscribes to the qmk_state_changed event, plus mouse.clicked (which
# just triggers a re-render of the current JSON — no daemon round-trip,
# because the daemon keeps the file fresh via push + 5s heartbeat).

set -eu

# shellcheck disable=SC1091
[ -f "$CONFIG_DIR/colors.sh" ] && source "$CONFIG_DIR/colors.sh"

STATE_FILE="${QMK_STATE_FILE:-/tmp/qmk_state.json}"

if [ ! -f "$STATE_FILE" ]; then
  sketchybar --set qmk_layer label="?" background.drawing=off \
             --set qmk_mods  label=""  background.drawing=off
  exit 0
fi

LAYER_NAME=$(jq -r '.top_layer_name // "?"' "$STATE_FILE")
MODS=$(jq -r '.mods_letters // ""' "$STATE_FILE")
MODS_STATE=$(jq -r '.mods_state // "none"' "$STATE_FILE")

case "$LAYER_NAME" in
  BASE|BASE-H) LC=${GREEN:-0xffa6da95} ;;
  FN)          LC=${ORANGE:-0xfff5a97f} ;;
  FN-H)        LC=${MAGENTA:-0xffc6a0f6} ;;
  NAS|NAS-H)   LC=${BLUE:-0xff8aadf4} ;;
  NUM)         LC=${RED:-0xffed8796} ;;
  MBO)         LC=${MAGENTA:-0xffc6a0f6} ;;
  *)           LC=${GREY:-0xff939ab7} ;;
esac

case "$MODS_STATE" in
  held)   MC=${RED:-0xffed8796} ;;
  weak)   MC=${ORANGE:-0xfff5a97f} ;;
  osm)    MC=${YELLOW:-0xffeed49f} ;;
  locked) MC=${BLUE:-0xff8aadf4} ;;
  *)      MC=${GREY:-0xff939ab7} ;;
esac

if [ -z "$MODS" ]; then
  MODS_LABEL=""
  MODS_DRAW=off
else
  MODS_LABEL="$MODS"
  MODS_DRAW=on
fi

sketchybar --set qmk_layer \
             label="$LAYER_NAME" \
             label.color=${WHITE:-0xffcad3f5} \
             background.color="$LC" \
             background.drawing=on \
           --set qmk_mods \
             label="$MODS_LABEL" \
             label.color=${WHITE:-0xffcad3f5} \
             background.color="$MC" \
             background.drawing="$MODS_DRAW"
