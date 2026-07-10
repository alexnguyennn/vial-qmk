//! HID transport abstraction: real hidapi impl for prod, mock for tests.

use anyhow::Result;

pub trait HidTransport: Send {
    /// Read one packet (blocking). Returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
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

    pub struct MockTransport {
        pub reads: VecDeque<Result<Vec<u8>>>,
        pub writes: Vec<Vec<u8>>,
    }

    impl MockTransport {
        pub fn new(reads: Vec<Result<Vec<u8>>>) -> Self {
            Self {
                reads: reads.into(),
                writes: Vec::new(),
            }
        }
    }

    impl HidTransport for MockTransport {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            match self.reads.pop_front() {
                Some(Ok(data)) => {
                    let n = data.len().min(buf.len());
                    buf[..n].copy_from_slice(&data[..n]);
                    Ok(n)
                }
                Some(Err(e)) => Err(e),
                None => anyhow::bail!("mock transport exhausted"),
            }
        }

        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            self.writes.push(buf.to_vec());
            Ok(buf.len())
        }
    }
}
