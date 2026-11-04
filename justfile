set shell := ["bash", "-euo", "pipefail", "-c"]

QMK_STATE_DAEMON_JUSTFILE := justfile_directory() + "/tools/qmk-state-daemon/justfile"
SVALBOARD_TOOLS_JUSTFILE := justfile_directory() + "/keyboards/svalboard/keymaps/alex/tools/justfile"

default:
    @just --list

# Forward recipes into tools/qmk-state-daemon.
daemon *args:
    just --justfile "{{ QMK_STATE_DAEMON_JUSTFILE }}" {{ args }}

# Forward recipes into keyboards/svalboard/keymaps/alex/tools.
svalboard *args:
    just --justfile "{{ SVALBOARD_TOOLS_JUSTFILE }}" {{ args }}
