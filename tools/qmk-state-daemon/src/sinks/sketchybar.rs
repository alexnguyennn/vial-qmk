//! macOS sketchybar event sink.

use std::process::Command;

use anyhow::Result;

use crate::sink::{EventArg, EventSink};

pub struct SketchybarSink;

pub fn sketchybar_trigger_args(event: &str, args: &[EventArg]) -> Vec<String> {
    let mut out = vec!["--trigger".to_string(), event.to_string()];
    for arg in args {
        out.push(format!("{}={}", arg.key, arg.value));
    }
    out
}

impl EventSink for SketchybarSink {
    fn emit(&self, event: &str, args: &[EventArg], _payload_json: &str) -> Result<()> {
        let argv = sketchybar_trigger_args(event, args);
        let status = Command::new("sketchybar").args(argv).status()?;
        if !status.success() {
            anyhow::bail!("sketchybar --trigger {event} exited {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sketchybar_trigger_args() {
        let args = vec![
            EventArg::new("top_layer_name", "BASE"),
            EventArg::new("mods_letters", "CS"),
        ];
        assert_eq!(
            sketchybar_trigger_args("qmk_state_changed", &args),
            vec![
                "--trigger",
                "qmk_state_changed",
                "top_layer_name=BASE",
                "mods_letters=CS",
            ]
        );
    }
}
