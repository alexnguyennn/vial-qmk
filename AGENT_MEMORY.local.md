# Agent Memory

- `tools/qmk-state-daemon` owns the raw-HID interface exclusively; use `qmk-state-daemon qsid ...` for flow-tap QSID reads/writes while the daemon is running. The Python fallback `keyboards/svalboard/keymaps/alex/tools/vial-qs.py` only works after stopping the launchd agent.
- Broker loop must use HID `read_timeout()` rather than blocking `read()`; blocking reads stall both sketchybar state updates and QSID RPC while the board is idle.
- After daemon recovery, `sketchybar --reload` may be needed to refresh the qmk items because dedup suppresses unchanged heartbeats and the bar can miss the daemon's initial event if it reloads later.
