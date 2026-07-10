//! Sketchybar event trigger abstraction.

use std::process::Command;

use anyhow::Result;

pub trait Notifier: Send {
    fn notify(&self, event: &str) -> Result<()>;
}

pub struct SketchybarNotifier;

impl Notifier for SketchybarNotifier {
    fn notify(&self, event: &str) -> Result<()> {
        let status = Command::new("sketchybar")
            .args(["--trigger", event])
            .status()?;
        if !status.success() {
            anyhow::bail!("sketchybar --trigger {event} exited {status}");
        }
        Ok(())
    }
}

pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn notify(&self, _event: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub mod recording {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub struct RecordingNotifier {
        pub events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingNotifier {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl Notifier for RecordingNotifier {
        fn notify(&self, event: &str) -> Result<()> {
            self.events.lock().unwrap().push(event.to_string());
            Ok(())
        }
    }
}
