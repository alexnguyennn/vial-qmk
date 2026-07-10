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
use crate::sketchybar::Notifier;
use crate::transport::HidTransport;

pub const PACKET_LEN: usize = STATE_PACKET_LEN;

pub struct RunOptions {
    pub sketchybar_event: String,
    pub once: bool,
    /// Backoff between failed reads. `None` = default 100ms with 1s cap.
    pub backoff: Option<Duration>,
    pub max_backoff: Option<Duration>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            sketchybar_event: "qmk_state_changed".to_string(),
            once: false,
            backoff: None,
            max_backoff: None,
        }
    }
}

/// Run the main read → decode → write → notify loop.
///
/// Firmware pushes 0xAB state packets on every layer/mod change plus a
/// periodic heartbeat, so the daemon is purely push-driven. Returns when
/// `once` is true and a packet was successfully processed.
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
                    writer.write(&value)?;
                    notifier.notify(&opts.sketchybar_event)?;
                }
                if opts.once {
                    return Ok(());
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
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(notifier.events(), vec!["e"]);
    }
}
