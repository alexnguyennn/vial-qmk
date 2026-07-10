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

pub const QUERY_MSG_ID: u8 = 0xAC;
pub const QUERY_VERSION: u8 = 1;
pub const PACKET_LEN: usize = STATE_PACKET_LEN;

/// Build a full 32-byte query request packet.
pub fn build_query_packet() -> [u8; PACKET_LEN] {
    let mut buf = [0u8; PACKET_LEN];
    buf[0] = QUERY_MSG_ID;
    buf[1] = QUERY_VERSION;
    buf
}

/// Send a query packet through the transport.
pub fn send_query<T: HidTransport>(transport: &mut T) -> Result<()> {
    let pkt = build_query_packet();
    transport.write(&pkt)?;
    Ok(())
}

pub struct RunOptions {
    pub sketchybar_event: String,
    pub query_on_start: bool,
    pub once: bool,
    /// Backoff between failed reads. `None` = default 100ms with 1s cap.
    pub backoff: Option<Duration>,
    pub max_backoff: Option<Duration>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            sketchybar_event: "qmk_state_changed".to_string(),
            query_on_start: true,
            once: false,
            backoff: None,
            max_backoff: None,
        }
    }
}

/// Run the main read → decode → write → notify loop.
///
/// Returns when `once` is true and a packet was successfully processed,
/// or when `transport.read` returns a non-retryable error path exhausts.
pub fn run<T: HidTransport, W: StateWriter, N: Notifier>(
    transport: &mut T,
    registry: &Registry,
    writer: &mut W,
    notifier: &N,
    opts: &RunOptions,
) -> Result<()> {
    if opts.query_on_start {
        // Best-effort; a fresh device may not be ready to receive yet.
        let _ = send_query(transport);
    }

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
            query_on_start: true,
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

        // Query written on start.
        assert_eq!(transport.writes.len(), 1);
        assert_eq!(transport.writes[0][0], QUERY_MSG_ID);

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
            query_on_start: false,
            sketchybar_event: "e".into(),
            backoff: Some(Duration::from_millis(0)),
            max_backoff: Some(Duration::from_millis(0)),
        };
        run(&mut transport, &registry, &mut writer, &notifier, &opts).unwrap();

        assert_eq!(notifier.events(), vec!["e"]);
    }

    #[test]
    fn query_packet_layout() {
        let p = build_query_packet();
        assert_eq!(p.len(), PACKET_LEN);
        assert_eq!(p[0], QUERY_MSG_ID);
        assert_eq!(p[1], QUERY_VERSION);
        assert!(p[2..].iter().all(|&b| b == 0));
    }
}
