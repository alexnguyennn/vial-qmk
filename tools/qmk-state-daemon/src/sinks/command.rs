//! Generic command sink for Linux and other non-sketchybar consumers.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Result};

use crate::sink::{EventArg, EventSink};

#[derive(Debug, Clone)]
pub struct CommandSink {
    program: Vec<String>,
    state_file: PathBuf,
}

impl CommandSink {
    pub fn new(program: Vec<String>, state_file: impl Into<PathBuf>) -> Result<Self> {
        if program.is_empty() {
            bail!("command sink program must not be empty");
        }
        Ok(Self {
            program,
            state_file: state_file.into(),
        })
    }

    pub fn env_pairs(
        &self,
        event: &str,
        args: &[EventArg],
        payload_json: &str,
    ) -> Vec<(String, String)> {
        let mut out = vec![
            ("QMK_EVENT_NAME".to_string(), event.to_string()),
            (
                "QMK_STATE_JSON".to_string(),
                self.state_file.display().to_string(),
            ),
            (
                "QMK_STATE_PAYLOAD_JSON".to_string(),
                payload_json.to_string(),
            ),
        ];
        for arg in args {
            out.push((arg.env_key(), arg.value.clone()));
        }
        out
    }
}

impl EventSink for CommandSink {
    fn emit(&self, event: &str, args: &[EventArg], payload_json: &str) -> Result<()> {
        let mut cmd = Command::new(&self.program[0]);
        if self.program.len() > 1 {
            cmd.args(&self.program[1..]);
        }
        for (k, v) in self.env_pairs(event, args, payload_json) {
            cmd.env(k, v);
        }
        let status = cmd.status()?;
        if !status.success() {
            bail!("command sink exited {status}: {}", self.program.join(" "));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_program() {
        assert!(CommandSink::new(Vec::<String>::new(), "/tmp/state.json").is_err());
    }

    #[test]
    fn builds_env_pairs() {
        let sink = CommandSink::new(vec!["/bin/true".into()], "/tmp/qmk.json").unwrap();
        let args = vec![
            EventArg::new("top_layer_name", "BASE"),
            EventArg::new("mods_state", "held"),
        ];
        let env = sink.env_pairs("qmk_state_changed", &args, r#"{"top_layer_name":"BASE"}"#);
        assert!(env.contains(&("QMK_EVENT_NAME".into(), "qmk_state_changed".into())));
        assert!(env.contains(&("QMK_STATE_JSON".into(), "/tmp/qmk.json".into())));
        assert!(env.contains(&("QMK_TOP_LAYER_NAME".into(), "BASE".into())));
        assert!(env.contains(&(
            "QMK_STATE_PAYLOAD_JSON".into(),
            r#"{"top_layer_name":"BASE"}"#.into()
        )));
        assert!(env.contains(&("QMK_MODS_STATE".into(), "held".into())));
    }
}
