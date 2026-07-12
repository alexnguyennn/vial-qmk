#!/usr/bin/env sh
set -eu

out="${QMK_I3STATUS_FILE:-${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.i3status}"
layer="${QMK_TOP_LAYER_NAME:-?}"
mods="${QMK_MODS_LETTERS:-}"

if [ -n "$mods" ]; then
  text="$layer $mods"
else
  text="$layer"
fi

mkdir -p "$(dirname "$out")"
# Write the watched path in place so i3status-rust `watch_files`
# observes modifications without relying on inode replacement events.
printf '%s\n' "$text" > "$out"
