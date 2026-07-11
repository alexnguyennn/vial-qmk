//! Sketchybar event trigger abstraction.

use std::process::Command;

use anyhow::Result;

/// A single key=value pair passed as an event argument to
/// `sketchybar --trigger EVENT k1=v1 k2=v2 …`. Lua items receive
/// these as `event.k1` etc.
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
}

pub trait Notifier: Send {
    fn notify(&self, event: &str, args: &[EventArg]) -> Result<()>;
}

pub struct SketchybarNotifier;

impl Notifier for SketchybarNotifier {
    fn notify(&self, event: &str, args: &[EventArg]) -> Result<()> {
        let mut cmd = Command::new("sketchybar");
        cmd.arg("--trigger").arg(event);
        for arg in args {
            cmd.arg(format!("{}={}", arg.key, arg.value));
        }
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("sketchybar --trigger {event} exited {status}");
        }
        Ok(())
    }
}

pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn notify(&self, _event: &str, _args: &[EventArg]) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub mod recording {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedNotify {
        pub event: String,
        pub args: Vec<EventArg>,
    }

    #[derive(Clone, Default)]
    pub struct RecordingNotifier {
        pub calls: Arc<Mutex<Vec<RecordedNotify>>>,
    }

    impl RecordingNotifier {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn calls(&self) -> Vec<RecordedNotify> {
            self.calls.lock().unwrap().clone()
        }
        /// Convenience for older tests that only care about event names.
        pub fn events(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.event.clone())
                .collect()
        }
    }

    impl Notifier for RecordingNotifier {
        fn notify(&self, event: &str, args: &[EventArg]) -> Result<()> {
            self.calls.lock().unwrap().push(RecordedNotify {
                event: event.to_string(),
                args: args.to_vec(),
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_arg_new() {
        let a = EventArg::new("k", "v");
        assert_eq!(a.key, "k");
        assert_eq!(a.value, "v");
    }
}
