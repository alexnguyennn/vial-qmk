//! HID transport abstraction: real hidapi impl for prod, mock for tests.

use anyhow::Result;

pub trait HidTransport: Send {
    /// Read one packet (blocking). Returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    /// Read with a timeout in milliseconds. `timeout_ms == 0` returns
    /// immediately if no data is available (n = 0).
    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize>;
    /// Write one packet.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
}

pub struct HidApiTransport {
    device: hidapi::HidDevice,
}

impl HidApiTransport {
    /// Open the first HID device with the given VID/PID whose usage_page
    /// matches Vial's raw HID interface (0xFF60, usage 0x61).
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let api = hidapi::HidApi::new()?;
        for info in api.device_list() {
            if info.vendor_id() == vid
                && info.product_id() == pid
                && info.usage_page() == 0xFF60
                && info.usage() == 0x61
            {
                let device = info.open_device(&api)?;
                return Ok(Self { device });
            }
        }
        anyhow::bail!(
            "no raw-HID interface (usage_page 0xFF60, usage 0x61) found for {:04X}:{:04X}",
            vid,
            pid
        );
    }
}

impl HidTransport for HidApiTransport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.device.read(buf)?)
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize> {
        Ok(self.device.read_timeout(buf, timeout_ms)?)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        // QMK raw_hid on macOS expects a leading report id byte (0x00).
        let mut framed = Vec::with_capacity(buf.len() + 1);
        framed.push(0x00);
        framed.extend_from_slice(buf);
        Ok(self.device.write(&framed)?)
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::VecDeque;

    /// Test transport with two independent queues:
    /// - `unsolicited`: reads that will be returned by both `read` and
    ///   `read_timeout` even without a preceding write. Use for push
    ///   state packets.
    /// - `responses`: reads that will only be returned AFTER a `write`
    ///   call. Simulates request/response RPCs, so the broker's idle
    ///   loop can't prematurely consume them.
    pub struct MockTransport {
        pub unsolicited: VecDeque<Result<Vec<u8>>>,
        pub responses: VecDeque<Result<Vec<u8>>>,
        pub writes: Vec<Vec<u8>>,
        pending_responses: usize,
    }

    impl MockTransport {
        /// Legacy constructor: everything in `reads` is treated as
        /// unsolicited (backward-compatible with older tests).
        pub fn new(reads: Vec<Result<Vec<u8>>>) -> Self {
            Self {
                unsolicited: reads.into(),
                responses: VecDeque::new(),
                writes: Vec::new(),
                pending_responses: 0,
            }
        }

        /// New: queue write-gated response packets.
        pub fn with_responses(responses: Vec<Result<Vec<u8>>>) -> Self {
            Self {
                unsolicited: VecDeque::new(),
                responses: responses.into(),
                writes: Vec::new(),
                pending_responses: 0,
            }
        }
    }

    impl HidTransport for MockTransport {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.pending_responses > 0 {
                if let Some(item) = self.responses.pop_front() {
                    self.pending_responses -= 1;
                    return match item {
                        Ok(data) => {
                            let n = data.len().min(buf.len());
                            buf[..n].copy_from_slice(&data[..n]);
                            Ok(n)
                        }
                        Err(e) => Err(e),
                    };
                }
            }
            match self.unsolicited.pop_front() {
                Some(Ok(data)) => {
                    let n = data.len().min(buf.len());
                    buf[..n].copy_from_slice(&data[..n]);
                    Ok(n)
                }
                Some(Err(e)) => Err(e),
                None => anyhow::bail!("mock transport exhausted"),
            }
        }

        fn read_timeout(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize> {
            // When there's nothing queued at all, simulate a timeout.
            if self.pending_responses == 0 && self.unsolicited.is_empty() {
                return Ok(0);
            }
            self.read(buf)
        }

        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            self.writes.push(buf.to_vec());
            // Every write unlocks one response.
            if !self.responses.is_empty() {
                self.pending_responses += 1;
            }
            Ok(buf.len())
        }
    }
}
