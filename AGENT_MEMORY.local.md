# Agent Memory

- `tools/qmk-state-daemon` owns the raw-HID interface exclusively; use `qmk-state-daemon qsid ...` for flow-tap QSID reads/writes while the daemon is running. The Python fallback `keyboards/svalboard/keymaps/alex/tools/vial-qs.py` only works after stopping the launchd agent.
- There is no daemon-side suspend mode yet; pausing/resuming raw-HID ownership is done at the launchd layer (`just pause-launchd` / `just resume-launchd` in `tools/qmk-state-daemon`, or `daemon-pause` / `daemon-resume` in the keymap tools dir).
- Broker loop must use HID `read_timeout()` rather than blocking `read()`; blocking reads stall both sketchybar state updates and QSID RPC while the board is idle.
- After daemon recovery, `sketchybar --reload` may be needed to refresh the qmk items because dedup suppresses unchanged heartbeats and the bar can miss the daemon's initial event if it reloads later.
- `qmk-state-daemon` now uses XDG defaults for config/state/socket: `${XDG_CONFIG_HOME:-~/.config}/qmk-state-daemon/config.toml`, `${XDG_STATE_HOME:-~/.local/state}/qmk-state-daemon/state.json`, and `${XDG_RUNTIME_DIR:-/tmp}/qmk-state-daemon.sock`; keep client subcommand defaults aligned with daemon defaults.
- Linux support is via `just install-systemd-user`, `just install-udev-rule 303a 4044`, and generic `CommandSink`; `reload-config` only reloads sink/event/state-file, while VID/PID/socket changes require service restart.
