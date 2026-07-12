//! Local control plane: Unix socket RPC server + client, plus the
//! HID broker that owns the device and serializes access between the
//! push state pipeline and QSID request/response operations.
//!
//! Wire format: newline-delimited JSON over a Unix domain socket.
//! One request per line, one response per line. Simple, greppable,
//! easy to hand-drive with `nc -U`.
//!
//! Commands:
//! - `{"cmd":"qsid_list"}` → `{"ok":true,"qsids":[...]}`
//! - `{"cmd":"qsid_get","qsid":28,"width":2}` → `{"ok":true,"value":25}`
//! - `{"cmd":"qsid_set","qsid":28,"value":40,"width":2}` → `{"ok":true,"value":40}`
//! - `{"cmd":"ping"}` → `{"ok":true,"pong":true}`
//!
//! Width is optional when the QSID is in `vial_qsid::known_qsids`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::vial_qsid;

pub const DEFAULT_SOCKET_PATH: &str = "/tmp/qmk-state-daemon.sock";

/// A request coming in from a socket client, plus a channel to reply
/// on. The broker thread receives these, runs them against the HID
/// transport, and sends the response back.
pub struct BrokerRequest {
    pub packet: [u8; vial_qsid::PACKET_LEN],
    pub reply: Sender<Result<Vec<u8>>>,
}

pub type BrokerSender = Sender<BrokerRequest>;

/// Perform a QSID list against the broker (may need multiple round
/// trips because the response only carries a page of QSIDs at a
/// time).
pub fn qsid_list_via_broker(broker: &BrokerSender) -> Result<Vec<u16>> {
    let mut all: Vec<u16> = Vec::new();
    let mut cursor: u16 = 0;
    loop {
        let pkt = vial_qsid::build_query_packet(cursor);
        let resp = send_via_broker(broker, pkt)?;
        let (page, end) = vial_qsid::parse_query_response(&resp);
        let mut new_max = cursor;
        for q in &page {
            if !all.contains(q) {
                all.push(*q);
            }
            if *q > new_max {
                new_max = *q;
            }
        }
        if end {
            break;
        }
        if new_max == cursor {
            // Nothing new; guard against infinite loop.
            break;
        }
        cursor = new_max;
    }
    all.sort_unstable();
    Ok(all)
}

pub fn qsid_get_via_broker(broker: &BrokerSender, qsid: u16, width: usize) -> Result<u32> {
    let pkt = vial_qsid::build_get_packet(qsid);
    let resp = send_via_broker(broker, pkt)?;
    vial_qsid::parse_get_response(&resp, width)
}

pub fn qsid_set_via_broker(
    broker: &BrokerSender,
    qsid: u16,
    value: u32,
    width: usize,
) -> Result<u32> {
    let pkt = vial_qsid::build_set_packet(qsid, value, width)?;
    let resp = send_via_broker(broker, pkt)?;
    vial_qsid::parse_set_response(&resp)?;
    // Read back for verification.
    qsid_get_via_broker(broker, qsid, width)
}

fn send_via_broker(broker: &BrokerSender, packet: [u8; vial_qsid::PACKET_LEN]) -> Result<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    broker
        .send(BrokerRequest {
            packet,
            reply: tx,
        })
        .map_err(|_| anyhow!("broker channel closed"))?;
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|e| anyhow!("no response from broker: {e}"))?
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum RpcRequest {
    QsidList,
    QsidGet { qsid: u16, width: Option<usize> },
    QsidSet { qsid: u16, value: u32, width: Option<usize> },
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcResponse {
    Ok(serde_json::Value),
    Err { ok: bool, error: String },
}

impl RpcResponse {
    pub fn ok(v: serde_json::Value) -> Self {
        RpcResponse::Ok(v)
    }
    pub fn err(e: impl std::fmt::Display) -> Self {
        RpcResponse::Err {
            ok: false,
            error: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Server: bind socket, spawn per-connection handlers
// ---------------------------------------------------------------------------

pub struct Server {
    listener: UnixListener,
    broker: BrokerSender,
    path: PathBuf,
}

impl Server {
    pub fn bind(path: impl AsRef<Path>, broker: BrokerSender) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        // Remove stale socket.
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing stale socket {}", path.display()))?;
        }
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("binding {}", path.display()))?;
        Ok(Self { listener, broker, path })
    }

    /// Run the accept loop in the current thread. Blocks.
    pub fn serve(&self) -> Result<()> {
        for stream in self.listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("accept error: {e}");
                    continue;
                }
            };
            let broker = self.broker.clone();
            thread::spawn(move || {
                if let Err(e) = handle_client(stream, broker) {
                    eprintln!("client error: {e}");
                }
            });
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle_client(stream: UnixStream, broker: BrokerSender) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }
        let resp = match serde_json::from_str::<RpcRequest>(trimmed) {
            Ok(req) => dispatch(&broker, req),
            Err(e) => RpcResponse::err(format!("parse error: {e}")),
        };
        let mut buf = serde_json::to_string(&resp)?;
        buf.push('\n');
        writer.write_all(buf.as_bytes())?;
        line.clear();
    }
    Ok(())
}

fn dispatch(broker: &BrokerSender, req: RpcRequest) -> RpcResponse {
    let result: Result<serde_json::Value> = match req {
        RpcRequest::Ping => Ok(serde_json::json!({"ok": true, "pong": true})),
        RpcRequest::QsidList => {
            qsid_list_via_broker(broker).map(|qsids| {
                let known = vial_qsid::known_qsids();
                let entries: Vec<_> = qsids
                    .iter()
                    .map(|q| {
                        let (name, width) = known
                            .get(q)
                            .map(|(n, w)| (Some(*n), Some(*w)))
                            .unwrap_or((None, None));
                        serde_json::json!({
                            "qsid": q,
                            "name": name,
                            "width": width,
                        })
                    })
                    .collect();
                serde_json::json!({"ok": true, "qsids": entries})
            })
        }
        RpcRequest::QsidGet { qsid, width } => {
            let w = vial_qsid::resolve_width(qsid, width);
            match w {
                Ok(w) => qsid_get_via_broker(broker, qsid, w)
                    .map(|v| serde_json::json!({"ok": true, "qsid": qsid, "value": v})),
                Err(e) => Err(e),
            }
        }
        RpcRequest::QsidSet { qsid, value, width } => {
            let w = vial_qsid::resolve_width(qsid, width);
            match w {
                Ok(w) => qsid_set_via_broker(broker, qsid, value, w)
                    .map(|v| serde_json::json!({"ok": true, "qsid": qsid, "value": v})),
                Err(e) => Err(e),
            }
        }
    };
    match result {
        Ok(v) => RpcResponse::ok(v),
        Err(e) => RpcResponse::err(e),
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    pub fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let stream = UnixStream::connect(path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, reader })
    }

    pub fn call(&mut self, req: &RpcRequest) -> Result<serde_json::Value> {
        let mut buf = serde_json::to_string(req)?;
        buf.push('\n');
        self.stream.write_all(buf.as_bytes())?;
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let resp: RpcResponse = serde_json::from_str(line.trim())
            .with_context(|| format!("parsing response: {line}"))?;
        match resp {
            RpcResponse::Ok(v) => Ok(v),
            RpcResponse::Err { error, .. } => bail!("{error}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Broker (HID owner): read from transport, route packets, service
// QSID requests.
// ---------------------------------------------------------------------------

/// Spawn the broker thread. Returns a `BrokerSender` for QSID RPC
/// use, and joins the state-packet handling logic via the provided
/// closure. The closure receives one full 32-byte state packet at a
/// time.
///
/// The broker owns the HID transport. Non-state packets (first byte
/// != 0xAB) that arrive without an outstanding request are dropped.
pub fn spawn_broker<T, F>(
    mut transport: T,
    mut on_state_packet: F,
) -> (BrokerSender, thread::JoinHandle<()>)
where
    T: crate::transport::HidTransport + 'static,
    F: FnMut(&[u8; vial_qsid::PACKET_LEN]) + Send + 'static,
{
    let (tx, rx): (BrokerSender, Receiver<BrokerRequest>) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut buf = [0u8; vial_qsid::PACKET_LEN];
        loop {
            // Prefer outbound RPC requests when they're ready; otherwise
            // do a short read for state packets so the loop stays
            // responsive to future RPC traffic.
            match rx.try_recv() {
                Ok(req) => {
                    let result = perform_qsid_request(&mut transport, &req.packet, &mut buf);
                    let _ = req.reply.send(result);
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            match transport.read_timeout(&mut buf, 100) {
                Ok(0) => {
                    // Timeout expired with no data; loop back to
                    // check for RPC requests.
                    continue;
                }
                Ok(_n) => {
                    if !buf.is_empty() && buf[0] == 0xAB {
                        on_state_packet(&buf);
                    }
                    // Drop non-state, non-solicited packets.
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    (tx, handle)
}

fn perform_qsid_request<T: crate::transport::HidTransport>(
    transport: &mut T,
    packet: &[u8; vial_qsid::PACKET_LEN],
    buf: &mut [u8; vial_qsid::PACKET_LEN],
) -> Result<Vec<u8>> {
    transport.write(packet).context("qsid write")?;
    // Read until we see a non-state packet (i.e. QSID response). Use a
    // bounded timeout so we don't hang if the device never responds.
    for _ in 0..32 {
        let n = transport
            .read_timeout(buf, 500)
            .context("qsid read")?;
        if n == 0 {
            continue;
        }
        if buf[0] == 0xAB {
            // State packet arrived interleaved with our response.
            // Drop it; the next real event or heartbeat will refresh.
            continue;
        }
        return Ok(buf[..n].to_vec());
    }
    bail!("no qsid response after 32 packet reads")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock::MockTransport;

    fn qsid_ok_response(value: u16) -> Vec<u8> {
        let mut r = vec![0u8; vial_qsid::PACKET_LEN];
        r[0] = 0;
        let b = value.to_le_bytes();
        r[1] = b[0];
        r[2] = b[1];
        r
    }

    #[test]
    fn broker_serves_qsid_get() {
        let transport = MockTransport::with_responses(vec![Ok(qsid_ok_response(25))]);
        let (tx, _h) = spawn_broker(transport, |_| {});
        let val = qsid_get_via_broker(&tx, 28, 2).unwrap();
        assert_eq!(val, 25);
    }

    #[test]
    fn broker_drops_interleaved_state_packet() {
        // Push one unsolicited state packet AND one response.
        // The QSID request should ignore the state packet (delivered
        // via the pre-write drain? no — actually the broker loop reads
        // it in the idle path; then the RPC comes in and writes,
        // which unlocks the response). Either way the QSID caller
        // must see the response value.
        let mut state = vec![0u8; vial_qsid::PACKET_LEN];
        state[0] = 0xAB;
        state[3] = 4;
        let mut transport = MockTransport::with_responses(vec![Ok(qsid_ok_response(42))]);
        transport.unsolicited.push_back(Ok(state));

        let (tx, _h) = spawn_broker(transport, |_| {});
        // Sleep briefly so the broker idle loop consumes the state
        // packet before the RPC arrives — mirrors production ordering
        // where state pushes precede user actions.
        std::thread::sleep(Duration::from_millis(50));
        let val = qsid_get_via_broker(&tx, 28, 2).unwrap();
        assert_eq!(val, 42);
    }

    #[test]
    fn broker_qsid_set_reads_back() {
        // set write -> ack, get write -> value 40. Both are
        // write-gated responses.
        let transport = MockTransport::with_responses(vec![
            Ok(qsid_ok_response(0)),  // set ack
            Ok(qsid_ok_response(40)), // read-back
        ]);
        let (tx, _h) = spawn_broker(transport, |_| {});
        let v = qsid_set_via_broker(&tx, 28, 40, 2).unwrap();
        assert_eq!(v, 40);
    }

    #[test]
    fn rpc_ping_roundtrip_over_socket() {
        // Set up broker with a mock that never returns anything;
        // ping doesn't touch it.
        let transport = MockTransport::new(vec![]);
        let (tx, _h) = spawn_broker(transport, |_| {});

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        let server = Server::bind(&sock, tx).unwrap();
        let sock_path = sock.clone();
        thread::spawn(move || {
            let _ = server.serve();
        });
        thread::sleep(Duration::from_millis(50));

        let mut client = Client::connect(&sock_path).unwrap();
        let resp = client.call(&RpcRequest::Ping).unwrap();
        assert_eq!(resp["pong"], true);
    }
}
