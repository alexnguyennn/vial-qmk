use anyhow::Result;

use crate::sink::{EventArg, EventSink};

pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: &str, _args: &[EventArg], _payload_json: &str) -> Result<()> {
        Ok(())
    }
}
