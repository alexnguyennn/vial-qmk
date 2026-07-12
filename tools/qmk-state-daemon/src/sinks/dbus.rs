//! D-Bus sink for i3status-rust `custom_dbus` blocks.
//!
//! i3status-rust owns the D-Bus object. This sink is a client that calls
//! `SetText` and optionally `SetIcon` on that object for each deduped
//! keyboard state change.

use std::collections::HashMap;

use anyhow::Result;
use zbus::blocking::{Connection, Proxy};

use crate::sink::{EventArg, EventSink};

pub const DEFAULT_DBUS_SERVICE: &str = "rs.i3status";
pub const DEFAULT_DBUS_PATH: &str = "/qmk_state";
pub const DEFAULT_DBUS_INTERFACE: &str = "rs.i3status.custom";
pub const DEFAULT_DBUS_TEXT: &str = "{top_layer_name} {mods_letters}";
pub const DEFAULT_DBUS_SHORT_TEXT: &str = "{top_layer_name}";

pub struct DbusSink {
    conn: Connection,
    service: String,
    path: String,
    interface: String,
    text: String,
    short_text: String,
    icon: Option<String>,
}

impl DbusSink {
    pub fn new(config: DbusSinkConfig) -> Result<Self> {
        Ok(Self {
            conn: Connection::session()?,
            service: config.service,
            path: config.path,
            interface: config.interface,
            text: config.text,
            short_text: config.short_text,
            icon: config.icon,
        })
    }

    pub fn rendered_text(
        &self,
        event: &str,
        args: &[EventArg],
        payload_json: &str,
    ) -> (String, String) {
        let values = template_values(event, args, payload_json);
        (
            expand_template(&self.text, &values),
            expand_template(&self.short_text, &values),
        )
    }

    pub fn target(&self) -> (&str, &str, &str) {
        (&self.service, &self.path, &self.interface)
    }
}

#[derive(Debug, Clone)]
pub struct DbusSinkConfig {
    pub service: String,
    pub path: String,
    pub interface: String,
    pub text: String,
    pub short_text: String,
    pub icon: Option<String>,
}

impl Default for DbusSinkConfig {
    fn default() -> Self {
        Self {
            service: DEFAULT_DBUS_SERVICE.to_string(),
            path: DEFAULT_DBUS_PATH.to_string(),
            interface: DEFAULT_DBUS_INTERFACE.to_string(),
            text: DEFAULT_DBUS_TEXT.to_string(),
            short_text: DEFAULT_DBUS_SHORT_TEXT.to_string(),
            icon: None,
        }
    }
}

impl EventSink for DbusSink {
    fn emit(&self, event: &str, args: &[EventArg], payload_json: &str) -> Result<()> {
        let (text, short_text) = self.rendered_text(event, args, payload_json);
        let proxy = Proxy::new(
            &self.conn,
            self.service.as_str(),
            self.path.as_str(),
            self.interface.as_str(),
        )?;
        if let Some(icon) = &self.icon {
            let _: String = proxy.call("SetIcon", &(icon.as_str(),))?;
        }
        let _: String = proxy.call("SetText", &(text.as_str(), short_text.as_str()))?;
        Ok(())
    }
}

fn template_values(event: &str, args: &[EventArg], payload_json: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    values.insert("event_name".to_string(), event.to_string());
    values.insert("QMK_EVENT_NAME".to_string(), event.to_string());
    values.insert("state_payload_json".to_string(), payload_json.to_string());
    values.insert(
        "QMK_STATE_PAYLOAD_JSON".to_string(),
        payload_json.to_string(),
    );
    for arg in args {
        values.insert(arg.key.clone(), arg.value.clone());
        values.insert(arg.env_key(), arg.value.clone());
    }
    values
}

fn expand_template(raw: &str, values: &HashMap<String, String>) -> String {
    let mut out = raw.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_targets_i3status_rust() {
        let cfg = DbusSinkConfig::default();
        assert_eq!(cfg.service, "rs.i3status");
        assert_eq!(cfg.path, "/qmk_state");
        assert_eq!(cfg.interface, "rs.i3status.custom");
    }

    #[test]
    fn renders_text_templates() {
        let cfg = DbusSinkConfig {
            text: "{top_layer_name} {mods_letters}".to_string(),
            short_text: "{QMK_TOP_LAYER_NAME}".to_string(),
            ..Default::default()
        };
        let values = template_values(
            "qmk_state_changed",
            &[
                EventArg::new("top_layer_name", "BASE"),
                EventArg::new("mods_letters", "CS"),
            ],
            "{}",
        );
        let text = expand_template(&cfg.text, &values);
        let short = expand_template(&cfg.short_text, &values);
        assert_eq!(text, "BASE CS");
        assert_eq!(short, "BASE");
    }
}
