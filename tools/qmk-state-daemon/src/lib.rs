//! qmk-state-daemon library crate.
//!
//! See `PLAN_flow_tap_shift.md` (Workstream B) for the full design.

pub mod handlers;
pub mod layer_names;
pub mod mods;
pub mod output;
pub mod packet;
pub mod sketchybar;
pub mod transport;

use std::time::Duration;

use anyhow::Result;

use crate::handlers::state::{STATE_MSG_ID, STATE_PACKET_LEN};
use crate::output::StateWriter;
use crate::packet::Registry;
use crate::sketchybar::{EventArg, Notifier};
use crate::transport::HidTransport;

pub const PACKET_LEN: usize = STATE_PACKET_LEN;

pub struct RunOptions {
    pub sketchybar_event: String,
    pub once: bool,
    /// Backoff between failed reads. `None` = default 100ms with 1s cap.
    pub backoff: Option<Duration>,
    pub max_backoff: Option<Duration>,
    /// Test-only cap: stop after this many successful reads. `None` = run
    /// forever (production behavior).
    pub max_reads: Option<usize>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            sketchybar_event: "qmk_state_changed".to_string(),
            once: false,
            backoff: None,
            max_backoff: None,
            max_reads: None,
        }
    }
}

/// Fields whose changes should NOT be treated as state changes for
/// dedup purposes. The firmware's 5s heartbeat sets a different reason
/// byte even when the observable state is identical, so we exclude
/// `reason` and `reason_flags` before comparing successive payloads.
const DEDUP_IGNORED_FIELDS: &[&str] = &["reason", "reason_flags"];

fn dedup_key(value: &serde_json::Value) -> serde_json::Value {
    let mut clone = value.clone();
    if let Some(obj) = clone.as_object_mut() {
        for field in DEDUP_IGNORED_FIELDS {
            obj.remove(*field);
        }
    }
    clone
}

/// Extract the sketchybar event args from a decoded state payload.
/// Only stable, display-relevant fields are forwarded — internal
/// bitmasks stay in the JSON file for debugging.
fn event_args_from(value: &serde_json::Value) -> Vec<EventArg> {
    const KEYS: &[&str] = &[
        "top_layer_name",
        "default_layer_name",
        "mods_letters",
        "mods_state",
    ];
    let mut out = Vec::with_capacity(KEYS.len());
    if let Some(obj) = value.as_object() {
        for key in KEYS {
            if let Some(v) = obj.get(*key) {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push(EventArg::new(*key, s));
            }
        }
    }
    out
}

/// Run the main read → decode → write → notify loop.
///
/// Firmware pushes 0xAB state packets on every layer/mod change plus a
/// periodic heartbeat, so the daemon is purely push-driven. The daemon
/// dedupes payloads (ignoring the volatile `reason`/`reason_flags`
/// fields) so heartbeats that carry the same observable state neither
/// rewrite the JSON file nor fire the sketchybar trigger.
///
/// Returns when `once` is true and a packet was successfully processed,
/// or when `max_reads` is reached (test-only cap).
pub fn run<T: HidTransport, W: StateWriter, N: Notifier>(
    transport: &mut T,
    registry: &Registry,
    writer: &mut W,
    notifier: &N,
    opts: &RunOptions,
) -> Result<()> {
    let initial_backoff = opts.backoff.unwrap_or(Duration::from_millis(100));
    let max_backoff = opts.max_backoff.unwrap_or(Duration::from_secs(1));
    let mut backoff = initial_backoff;

    let mut buf = [0u8; PACKET_LEN];
    let mut last_key: Option<serde_json::Value> = None;
    let mut reads_done: usize = 0;
    loop {
        match transport.read(&mut buf) {
            Ok(0) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(max_backoff);
                continue;
            }
            Ok(n) => {
                backoff = initial_backoff;
                if let Some(value) = registry.handle(&buf[..n])? {
                    let key = dedup_key(&value);
                    let changed = match &last_key {
                        Some(prev) => prev != &key,
                        None => true,
                    };
                    if changed {
                        writer.write(&value)?;
                        let args = event_args_from(&value);
                        notifier.notify(&opts.sketchybar_event, &args)?;
                        last_key = Some(key);
                    }
                }
                reads_done += 1;
                if opts.once {
                    return Ok(());
                }
                if let Some(limit) = opts.max_reads {
                    if reads_done >= limit {
                        return Ok(());
                    }
                }
            }
            Err(_) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(max_backoff);
                // Keep looping; upstream layer will reopen the device
                // if needed. For now transport errors are treated as
                // transient (device unplugged, etc.).
                continue;
            }
        }
    }
}

/// Convenience: does the msg_id match the state handler?
pub fn is_state_packet(buf: &[u8]) -> bool {
    buf.first().copied() == Some(STATE_MSG_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::state::{StateHandler, REASON_INITIAL, STATE_VERSION};
    use crate::mods::MOD_LSFT;
    use crate::output::StateWriter;
    use crate::packet::PacketHandler;
    use crate::sketchybar::recording::RecordingNotifier;
    use crate::transport::mock::MockTransport;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct RecordingWriter {
        pub values: Arc<Mutex<Vec<Value>>>,
    }
    impl StateWriter for RecordingWriter {
        fn write(&mut self, value: &Value) -> Result<()> {
            self.values.lock().unwrap().push(value.clone());
            Ok(())
        }
    }

    fn state_packet() -> Vec<u8> {
        let mut buf = vec![0u8; PACKET_LEN];
        buf[0] = STATE_MSG_ID;
        buf[1] = STATE_VERSION;
        buf[2] = REASON_INITIAL;
        buf[3] = 2; // FN
        buf[9] = MOD_LSFT;
        buf
    }

    #[test]
    fn e2e_once_with_mock_transport() {
        let mut transport = MockTransport::new(vec![Ok(state_packet())]);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let writer = RecordingWriter::default();
        let mut writer_clone = writer.clone();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: true,
            sketchybar_event: "qmk_state_changed".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: None,
        };
        run(
            &mut transport,
            &registry,
            &mut writer_clone,
            &notifier,
            &opts,
        )
        .unwrap();

        // Push-only design: no writes expected.
        assert!(transport.writes.is_empty());

        // One decoded payload written.
        let values = writer.values.lock().unwrap().clone();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["top_layer_name"], "FN");
        assert_eq!(values[0]["mods_letters"], "S");

        // Notifier fired once.
        assert_eq!(notifier.events(), vec!["qmk_state_changed"]);
    }

    #[test]
    fn recovers_from_transient_read_error() {
        let reads: Vec<Result<Vec<u8>>> = vec![
            Err(anyhow::anyhow!("transient")),
            Err(anyhow::anyhow!("transient again")),
            Ok(state_packet()),
        ];
        let mut transport = MockTransport::new(reads);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: true,
            sketchybar_event: "e".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: None,
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(notifier.events(), vec!["e"]);
    }

    fn heartbeat_packet_matching(seed: &[u8]) -> Vec<u8> {
        // Same observable state as `seed`, but reason byte differs
        // (INITIAL vs a change reason). Simulates the firmware's 5s
        // heartbeat re-broadcasting an unchanged snapshot.
        let mut buf = seed.to_vec();
        buf[2] = REASON_INITIAL;
        buf
    }

    fn changed_packet(seed: &[u8]) -> Vec<u8> {
        let mut buf = seed.to_vec();
        buf[3] = 4; // top_layer flips from FN to NAS
        buf
    }

    #[test]
    fn dedups_notifier_when_payload_unchanged() {
        // Three reads: initial state, then a heartbeat with identical
        // observable state, then a real change. Notifier should fire
        // exactly twice: once for the initial packet, once for the
        // real change.
        let seed = state_packet();
        let reads: Vec<Result<Vec<u8>>> = vec![
            Ok(seed.clone()),
            Ok(heartbeat_packet_matching(&seed)),
            Ok(changed_packet(&seed)),
        ];
        let mut transport = MockTransport::new(reads);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: false,
            sketchybar_event: "qmk_state_changed".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: Some(3),
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(
            notifier.events(),
            vec!["qmk_state_changed", "qmk_state_changed"],
            "heartbeat with identical observable state should not fire notifier"
        );
    }

    #[test]
    fn dedups_writer_when_payload_unchanged() {
        // Same scenario as above; writer should also skip the redundant
        // heartbeat to avoid pointless disk writes.
        let seed = state_packet();
        let reads: Vec<Result<Vec<u8>>> = vec![
            Ok(seed.clone()),
            Ok(heartbeat_packet_matching(&seed)),
            Ok(changed_packet(&seed)),
        ];
        let mut transport = MockTransport::new(reads);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: false,
            sketchybar_event: "qmk_state_changed".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: Some(3),
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(writer.values.lock().unwrap().len(), 2);
    }

    #[test]
    fn dedup_key_strips_reason_and_flags() {
        let mut buf = state_packet();
        buf[2] = REASON_INITIAL;
        let v1 = StateHandler::new().decode(&buf).unwrap();
        buf[2] = crate::handlers::state::REASON_LAYER
            | crate::handlers::state::REASON_MODS;
        let v2 = StateHandler::new().decode(&buf).unwrap();

        assert_ne!(v1, v2, "raw payloads should differ (reason bytes)");
        assert_eq!(
            dedup_key(&v1),
            dedup_key(&v2),
            "dedup keys should be equal after stripping reason/reason_flags"
        );
        let stripped = dedup_key(&v1);
        assert!(stripped.get("reason").is_none());
        assert!(stripped.get("reason_flags").is_none());
        assert!(stripped.get("top_layer_name").is_some());
        assert!(stripped.get("mods_letters").is_some());
    }

    #[test]
    fn dedups_multiple_consecutive_heartbeats() {
        // Three identical heartbeats after the initial packet. Only
        // the initial should fire the writer/notifier.
        let seed = state_packet();
        let reads: Vec<Result<Vec<u8>>> = vec![
            Ok(seed.clone()),
            Ok(heartbeat_packet_matching(&seed)),
            Ok(heartbeat_packet_matching(&seed)),
            Ok(heartbeat_packet_matching(&seed)),
        ];
        let mut transport = MockTransport::new(reads);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: false,
            sketchybar_event: "e".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: Some(4),
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(notifier.events(), vec!["e"]);
        assert_eq!(writer.values.lock().unwrap().len(), 1);
    }

    #[test]
    fn resumes_firing_after_change_between_dupes() {
        // A → A (dup) → B → B (dup) → A. Notifier should fire 3
        // times: initial A, change to B, change back to A.
        let a = state_packet();
        let b = changed_packet(&a);
        let reads: Vec<Result<Vec<u8>>> = vec![
            Ok(a.clone()),
            Ok(heartbeat_packet_matching(&a)),
            Ok(b.clone()),
            Ok(heartbeat_packet_matching(&b)),
            Ok(a.clone()),
        ];
        let mut transport = MockTransport::new(reads);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: false,
            sketchybar_event: "e".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: Some(5),
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(notifier.events().len(), 3);
        assert_eq!(writer.values.lock().unwrap().len(), 3);
    }

    #[test]
    fn detects_change_in_mods_only() {
        // Same top_layer, different mods. Must not be deduped.
        let a = state_packet(); // has MOD_LSFT
        let mut b = a.clone();
        b[9] = crate::mods::MOD_LCTL; // swap Shift for Ctrl
        let reads: Vec<Result<Vec<u8>>> = vec![Ok(a), Ok(b)];
        let mut transport = MockTransport::new(reads);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: false,
            sketchybar_event: "e".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: Some(2),
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        let values = writer.values.lock().unwrap().clone();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["mods_letters"], "S");
        assert_eq!(values[1]["mods_letters"], "C");
    }

    #[test]
    fn forwards_sketchybar_event_args_from_payload() {
        let mut transport = MockTransport::new(vec![Ok(state_packet())]);
        let registry = Registry::builder().handler(StateHandler::new()).build();
        let mut writer = RecordingWriter::default();
        let notifier = RecordingNotifier::new();

        let opts = RunOptions {
            once: true,
            sketchybar_event: "qmk_state_changed".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
            max_reads: None,
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        let calls = notifier.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].event, "qmk_state_changed");
        let args: std::collections::HashMap<_, _> = calls[0]
            .args
            .iter()
            .map(|a| (a.key.as_str(), a.value.as_str()))
            .collect();
        assert_eq!(args.get("top_layer_name").copied(), Some("FN"));
        assert_eq!(args.get("default_layer_name").copied(), Some("BASE"));
        assert_eq!(args.get("mods_letters").copied(), Some("S"));
        assert_eq!(args.get("mods_state").copied(), Some("held"));
    }
}
