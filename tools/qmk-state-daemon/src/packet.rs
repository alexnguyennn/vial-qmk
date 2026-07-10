//! Packet dispatch registry keyed by first-byte msg id.
//!
//! Extensibility: new HID msg ids get a new `PacketHandler` impl and
//! `Registry::builder().handler(...)` registration. No changes to
//! dispatch code needed.

use std::collections::HashMap;

use anyhow::{anyhow, Result};

pub trait PacketHandler: Send + Sync {
    fn msg_id(&self) -> u8;
    fn decode(&self, buf: &[u8]) -> Result<serde_json::Value>;
}

pub struct Registry {
    handlers: HashMap<u8, Box<dyn PacketHandler>>,
}

pub struct RegistryBuilder {
    handlers: HashMap<u8, Box<dyn PacketHandler>>,
}

impl Registry {
    pub fn builder() -> RegistryBuilder {
        RegistryBuilder {
            handlers: HashMap::new(),
        }
    }

    /// Dispatch a packet by its first byte. Returns `Ok(None)` if the
    /// buffer is empty; `Ok(Some(v))` for a known msg id; `Err` for an
    /// unknown msg id or decoder failure.
    pub fn handle(&self, buf: &[u8]) -> Result<Option<serde_json::Value>> {
        let Some(first) = buf.first() else {
            return Ok(None);
        };
        let handler = self
            .handlers
            .get(first)
            .ok_or_else(|| anyhow!("no handler for msg id 0x{:02X}", first))?;
        handler.decode(buf).map(Some)
    }

    pub fn known_ids(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.handlers.keys().copied().collect();
        v.sort_unstable();
        v
    }
}

impl RegistryBuilder {
    pub fn handler<H: PacketHandler + 'static>(mut self, h: H) -> Self {
        self.handlers.insert(h.msg_id(), Box::new(h));
        self
    }

    pub fn build(self) -> Registry {
        Registry {
            handlers: self.handlers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeHandler {
        id: u8,
        payload: serde_json::Value,
    }

    impl PacketHandler for FakeHandler {
        fn msg_id(&self) -> u8 {
            self.id
        }
        fn decode(&self, _buf: &[u8]) -> Result<serde_json::Value> {
            Ok(self.payload.clone())
        }
    }

    #[test]
    fn dispatches_by_msg_id() {
        let reg = Registry::builder()
            .handler(FakeHandler {
                id: 0xAB,
                payload: json!({"h": "ab"}),
            })
            .handler(FakeHandler {
                id: 0xAC,
                payload: json!({"h": "ac"}),
            })
            .build();

        assert_eq!(reg.handle(&[0xAB]).unwrap(), Some(json!({"h": "ab"})));
        assert_eq!(reg.handle(&[0xAC]).unwrap(), Some(json!({"h": "ac"})));
    }

    #[test]
    fn empty_buffer_is_none() {
        let reg = Registry::builder().build();
        assert!(reg.handle(&[]).unwrap().is_none());
    }

    #[test]
    fn unknown_id_is_err() {
        let reg = Registry::builder()
            .handler(FakeHandler {
                id: 0xAB,
                payload: json!({}),
            })
            .build();
        assert!(reg.handle(&[0x99]).is_err());
    }

    #[test]
    fn known_ids_sorted() {
        let reg = Registry::builder()
            .handler(FakeHandler {
                id: 0xAC,
                payload: json!({}),
            })
            .handler(FakeHandler {
                id: 0xAB,
                payload: json!({}),
            })
            .build();
        assert_eq!(reg.known_ids(), vec![0xAB, 0xAC]);
    }
}
