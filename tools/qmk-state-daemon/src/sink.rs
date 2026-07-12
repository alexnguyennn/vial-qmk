//! Event sink abstraction for state-change notifications.
//!
//! The daemon's HID/state pipeline is platform-neutral. Sinks are the
//! integration boundary where decoded keyboard state becomes a desktop
//! status update, script invocation, or no-op.

use anyhow::Result;

/// A single key=value pair emitted with a state-change event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventArg {
    pub key: String,
    pub value: String,
}

impl EventArg {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    pub fn env_key(&self) -> String {
        format!("QMK_{}", self.key.to_ascii_uppercase())
    }
}

pub trait EventSink: Send {
    fn emit(&self, event: &str, args: &[EventArg], payload_json: &str) -> Result<()>;
}

#[cfg(test)]
pub mod recording {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedEmit {
        pub event: String,
        pub args: Vec<EventArg>,
        pub payload_json: String,
    }

    #[derive(Clone, Default)]
    pub struct RecordingSink {
        pub calls: Arc<Mutex<Vec<RecordedEmit>>>,
    }

    impl RecordingSink {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<RecordedEmit> {
            self.calls.lock().unwrap().clone()
        }
        pub fn events(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.event.clone())
                .collect()
        }
    }

    impl EventSink for RecordingSink {
        fn emit(&self, event: &str, args: &[EventArg], payload_json: &str) -> Result<()> {
            self.calls.lock().unwrap().push(RecordedEmit {
                event: event.to_string(),
                args: args.to_vec(),
                payload_json: payload_json.to_string(),
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_arg_new_and_env_key() {
        let a = EventArg::new("top_layer_name", "BASE");
        assert_eq!(a.key, "top_layer_name");
        assert_eq!(a.value, "BASE");
        assert_eq!(a.env_key(), "QMK_TOP_LAYER_NAME");
    }
}
