//! Generic command sink for Linux and other non-sketchybar consumers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Result};

use crate::sink::{EventArg, EventSink};

#[derive(Debug, Clone)]
pub struct CommandSink {
    commands: Vec<Vec<String>>,
    state_file: PathBuf,
}

impl CommandSink {
    pub fn new(commands: Vec<Vec<String>>, state_file: impl Into<PathBuf>) -> Result<Self> {
        if commands.is_empty() {
            bail!("command sink commands must not be empty");
        }
        if commands.iter().any(Vec::is_empty) {
            bail!("command sink command entries must not be empty");
        }
        Ok(Self {
            commands,
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

    pub fn expanded_commands(
        &self,
        event: &str,
        args: &[EventArg],
        payload_json: &str,
    ) -> Vec<Vec<String>> {
        let env = self.env_pairs(event, args, payload_json);
        let values = template_values(&env);
        self.commands
            .iter()
            .map(|command| {
                command
                    .iter()
                    .map(|arg| expand_template(arg, &values))
                    .collect()
            })
            .collect()
    }
}

impl EventSink for CommandSink {
    fn emit(&self, event: &str, args: &[EventArg], payload_json: &str) -> Result<()> {
        let env = self.env_pairs(event, args, payload_json);
        let values = template_values(&env);
        for command in &self.commands {
            let expanded: Vec<_> = command
                .iter()
                .map(|arg| expand_template(arg, &values))
                .collect();
            let mut cmd = Command::new(&expanded[0]);
            if expanded.len() > 1 {
                cmd.args(&expanded[1..]);
            }
            for (k, v) in &env {
                cmd.env(k, v);
            }
            let status = cmd.status()?;
            if !status.success() {
                bail!("command sink exited {status}: {}", expanded.join(" "));
            }
        }
        Ok(())
    }
}

fn template_values(env: &[(String, String)]) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for (key, value) in env {
        values.insert(key.clone(), value.clone());
        if let Some(stripped) = key.strip_prefix("QMK_") {
            values.insert(stripped.to_ascii_lowercase(), value.clone());
        }
    }
    values
}

fn expand_template(raw: &str, values: &HashMap<String, String>) -> String {
    let mut out = raw.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_program() {
        assert!(CommandSink::new(Vec::<Vec<String>>::new(), "/tmp/state.json").is_err());
        assert!(CommandSink::new(vec![Vec::<String>::new()], "/tmp/state.json").is_err());
    }

    #[test]
    fn builds_env_pairs() {
        let sink = CommandSink::new(vec![vec!["/bin/true".into()]], "/tmp/qmk.json").unwrap();
        let args = vec![
            EventArg::new("top_layer_name", "BASE"),
            EventArg::new("mods_state", "held"),
            EventArg::new("caps_word_state", "active"),
            EventArg::new("caps_word_active", "true"),
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
        assert!(env.contains(&("QMK_CAPS_WORD_STATE".into(), "active".into())));
        assert!(env.contains(&("QMK_CAPS_WORD_ACTIVE".into(), "true".into())));
    }

    #[test]
    fn expands_template_args_for_each_command() {
        let sink = CommandSink::new(
            vec![
                vec![
                    "busctl".into(),
                    "SetText".into(),
                    "{top_layer_name} {mods_letters} {caps_word_state}".into(),
                    "{QMK_TOP_LAYER_NAME}".into(),
                ],
                vec!["busctl".into(), "SetIcon".into(), "keyboard".into()],
            ],
            "/tmp/qmk.json",
        )
        .unwrap();
        let args = vec![
            EventArg::new("top_layer_name", "BASE"),
            EventArg::new("mods_letters", "CS"),
            EventArg::new("caps_word_state", "active"),
        ];
        let commands = sink.expanded_commands("qmk_state_changed", &args, "{}");
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0][2], "BASE CS active");
        assert_eq!(commands[0][3], "BASE");
        assert_eq!(commands[1][2], "keyboard");
    }

    #[test]
    #[cfg(unix)]
    fn emit_runs_all_commands_with_same_env() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("qmk-command-sink-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out");
        let script_one = dir.join("one.sh");
        let script_two = dir.join("two.sh");
        fs::write(
            &script_one,
            "#!/bin/sh\nprintf 'one:%s:%s\\n' \"$QMK_TOP_LAYER_NAME\" \"$QMK_STATE_PAYLOAD_JSON\" >> \"$1\"\n",
        )
        .unwrap();
        fs::write(
            &script_two,
            "#!/bin/sh\nprintf 'two:%s:%s\\n' \"$QMK_TOP_LAYER_NAME\" \"$QMK_STATE_PAYLOAD_JSON\" >> \"$1\"\n",
        )
        .unwrap();
        fs::set_permissions(&script_one, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&script_two, fs::Permissions::from_mode(0o755)).unwrap();

        let sink = CommandSink::new(
            vec![
                vec![script_one.display().to_string(), out.display().to_string()],
                vec![script_two.display().to_string(), out.display().to_string()],
            ],
            "/tmp/qmk.json",
        )
        .unwrap();
        sink.emit(
            "qmk_state_changed",
            &[EventArg::new("top_layer_name", "BASE")],
            r#"{"top_layer_name":"BASE"}"#,
        )
        .unwrap();

        let written = fs::read_to_string(&out).unwrap();
        assert_eq!(
            written,
            "one:BASE:{\"top_layer_name\":\"BASE\"}\ntwo:BASE:{\"top_layer_name\":\"BASE\"}\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
