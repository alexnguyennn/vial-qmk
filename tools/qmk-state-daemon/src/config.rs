use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::rpc::DEFAULT_SOCKET_PATH;

pub const DEFAULT_EVENT_NAME: &str = "qmk_state_changed";
pub const DEFAULT_VID: u16 = 0x303A;
pub const DEFAULT_PID: u16 = 0x4044;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub vid: Option<String>,
    pub pid: Option<String>,
    pub state_file: Option<String>,
    pub socket: Option<String>,
    pub sink: Option<SinkConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SinkConfig {
    Sketchybar {
        event: Option<String>,
    },
    Command {
        event: Option<String>,
        program: Vec<String>,
    },
    Null {
        event: Option<String>,
    },
}

impl Default for SinkConfig {
    fn default() -> Self {
        #[cfg(target_os = "macos")]
        {
            SinkConfig::Sketchybar {
                event: Some(DEFAULT_EVENT_NAME.to_string()),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            SinkConfig::Null {
                event: Some(DEFAULT_EVENT_NAME.to_string()),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub vid: u16,
    pub pid: u16,
    pub state_file: PathBuf,
    pub socket: PathBuf,
    pub sink: SinkConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn resolve(self) -> Result<ResolvedConfig> {
        Ok(ResolvedConfig {
            vid: match parse_u16_auto(self.vid.as_deref()) {
                Some(v) => v?,
                None => DEFAULT_VID,
            },
            pid: match parse_u16_auto(self.pid.as_deref()) {
                Some(v) => v?,
                None => DEFAULT_PID,
            },
            state_file: resolve_path_opt(self.state_file.as_deref(), default_state_file()),
            socket: resolve_path_opt(self.socket.as_deref(), default_socket_path()),
            sink: self.sink.unwrap_or_default(),
        })
    }
}

pub fn default_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("qmk-state-daemon/config.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config/qmk-state-daemon/config.toml");
    }
    PathBuf::from("qmk-state-daemon.toml")
}

pub fn default_socket_path() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("qmk-state-daemon.sock");
    }
    PathBuf::from(DEFAULT_SOCKET_PATH)
}

pub fn default_state_file() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(xdg).join("qmk-state-daemon/state.json");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/state/qmk-state-daemon/state.json");
    }
    PathBuf::from("/tmp/qmk_state.json")
}

pub fn default_config_text() -> String {
    let sink = if cfg!(target_os = "macos") {
        r#"[sink]
kind = "sketchybar"
event = "qmk_state_changed"
"#
    } else {
        r#"[sink]
kind = "command"
event = "qmk_state_changed"
program = ["/bin/sh", "-lc", "env | grep '^QMK_' >> /tmp/qmk-state-daemon-events.log"]
"#
    };
    format!(
        r#"# qmk-state-daemon config
vid = "0x303A"
pid = "0x4044"
# Omit state_file/socket to use XDG defaults.
# state_file = "auto"
# socket = "auto"

{sink}"#
    )
}

pub fn parse_u16_auto(s: Option<&str>) -> Option<Result<u16>> {
    s.map(|s| {
        if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            Ok(u16::from_str_radix(rest, 16)?)
        } else {
            Ok(s.parse::<u16>()?)
        }
    })
}

fn resolve_path_opt(raw: Option<&str>, default: PathBuf) -> PathBuf {
    match raw {
        None | Some("") | Some("auto") => default,
        Some(path) => expand_home(path),
    }
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_and_decimal_u16() {
        assert_eq!(parse_u16_auto(Some("0x303A")).unwrap().unwrap(), 0x303A);
        assert_eq!(parse_u16_auto(Some("12346")).unwrap().unwrap(), 12346);
        assert!(parse_u16_auto(Some("70000")).unwrap().is_err());
    }

    #[test]
    fn parses_command_config() {
        let cfg: Config = toml::from_str(
            r#"
vid = "0x303A"
pid = "0x4044"
state_file = "/tmp/state.json"
socket = "/tmp/sock"

[sink]
kind = "command"
event = "e"
program = ["/bin/true"]
"#,
        )
        .unwrap();
        let resolved = cfg.resolve().unwrap();
        assert_eq!(resolved.vid, 0x303A);
        assert_eq!(resolved.pid, 0x4044);
        assert_eq!(resolved.state_file, PathBuf::from("/tmp/state.json"));
        assert_eq!(resolved.socket, PathBuf::from("/tmp/sock"));
        match resolved.sink {
            SinkConfig::Command { event, program } => {
                assert_eq!(event.as_deref(), Some("e"));
                assert_eq!(program, vec!["/bin/true"]);
            }
            _ => panic!("wrong sink"),
        }
    }

    #[test]
    fn default_config_contains_sink() {
        let text = default_config_text();
        assert!(text.contains("[sink]"));
        assert!(text.contains("qmk_state_changed"));
    }
}
