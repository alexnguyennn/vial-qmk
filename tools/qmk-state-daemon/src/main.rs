//! qmk-state-daemon CLI entry point.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use qmk_state_daemon::handlers::state::StateHandler;
use qmk_state_daemon::output::{FileStateWriter, StateWriter};
use qmk_state_daemon::packet::Registry;
use qmk_state_daemon::rpc::{
    self, Client, RpcRequest, Server, DEFAULT_SOCKET_PATH,
};
use qmk_state_daemon::sketchybar::{
    EventArg, Notifier, NullNotifier, SketchybarNotifier,
};
use qmk_state_daemon::transport::HidApiTransport;
use qmk_state_daemon::vial_qsid;

/// Default svalboard USB IDs (see keyboards/svalboard/info.json).
const DEFAULT_VID: u16 = 0x303A;
const DEFAULT_PID: u16 = 0x4044;

#[derive(Parser)]
#[command(name = "qmk-state-daemon", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon: read packets, write JSON, trigger sketchybar,
    /// and serve the local RPC socket.
    Run(RunArgs),
    /// List raw-HID capable HID devices with usage_page 0xFF60.
    List,
    /// Vial custom QSID commands (client → daemon RPC).
    #[command(subcommand)]
    Qsid(QsidCmd),
    /// Ping the running daemon over the RPC socket.
    Ping {
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
}

#[derive(Subcommand)]
enum QsidCmd {
    /// List all supported QSIDs on the connected keyboard.
    List {
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
    /// Read a QSID value.
    Get {
        qsid: u16,
        #[arg(long, value_parser = ["1", "2", "4"])]
        width: Option<String>,
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
    /// Write a QSID value; prints the read-back value on success.
    Set {
        qsid: u16,
        value: u32,
        #[arg(long, value_parser = ["1", "2", "4"])]
        width: Option<String>,
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
}

#[derive(Args, Clone)]
struct DeviceArgs {
    #[arg(long, value_parser = parse_hex_u16, default_value_t = DEFAULT_VID)]
    vid: u16,
    #[arg(long, value_parser = parse_hex_u16, default_value_t = DEFAULT_PID)]
    pid: u16,
}

#[derive(Args, Clone)]
struct RunArgs {
    #[command(flatten)]
    device: DeviceArgs,

    #[arg(long, default_value = "/tmp/qmk_state.json")]
    state_file: PathBuf,

    #[arg(long, default_value = "qmk_state_changed")]
    sketchybar_event: String,

    /// Log instead of invoking sketchybar.
    #[arg(long)]
    dry_run: bool,

    /// Path of the local RPC socket used by `qsid`/`ping` subcommands.
    #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
    socket: PathBuf,
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    // Accept "0x303A", "0X303A", or plain decimal "12346".
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(rest, 16).map_err(|e| e.to_string())
    } else {
        s.parse::<u16>().map_err(|e| e.to_string())
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run(args) => run_cmd(args),
        Cmd::List => list_cmd(),
        Cmd::Ping { socket } => ping_cmd(&socket),
        Cmd::Qsid(qs) => match qs {
            QsidCmd::List { socket } => qsid_list(&socket),
            QsidCmd::Get { qsid, width, socket } => qsid_get(&socket, qsid, parse_width(&width)?),
            QsidCmd::Set { qsid, value, width, socket } => {
                qsid_set(&socket, qsid, value, parse_width(&width)?)
            }
        },
    }
}

fn parse_width(w: &Option<String>) -> Result<Option<usize>> {
    Ok(w.as_deref().map(|s| s.parse::<usize>().unwrap()))
}

fn run_cmd(args: RunArgs) -> Result<()> {
    let transport = HidApiTransport::open(args.device.vid, args.device.pid)?;
    let registry = Registry::builder().handler(StateHandler::new()).build();
    let mut writer: Box<dyn StateWriter> =
        Box::new(FileStateWriter::new(args.state_file.clone()));
    let notifier: Box<dyn Notifier> = if args.dry_run {
        Box::new(NullNotifier)
    } else {
        Box::new(SketchybarNotifier)
    };
    let event = args.sketchybar_event.clone();
    let mut last_key: Option<serde_json::Value> = None;

    // State packet handler: decode, dedup, write JSON, fire notifier.
    let on_state = move |pkt: &[u8; vial_qsid::PACKET_LEN]| {
        let value = match registry.handle(pkt) {
            Ok(Some(v)) => v,
            _ => return,
        };
        let key = dedup_key(&value);
        if last_key.as_ref() == Some(&key) {
            return;
        }
        last_key = Some(key);
        if let Err(e) = writer.write(&value) {
            eprintln!("write state file error: {e}");
            return;
        }
        let ev_args = event_args_from(&value);
        if let Err(e) = notifier.notify(&event, &ev_args) {
            eprintln!("notifier error: {e}");
        }
    };

    let (broker, _broker_handle) = rpc::spawn_broker(transport, on_state);

    // Serve socket in current thread.
    let server = Server::bind(&args.socket, broker).context("bind rpc socket")?;
    println!(
        "qmk-state-daemon running: rpc socket = {}",
        args.socket.display()
    );
    server.serve()?;
    Ok(())
}

fn list_cmd() -> Result<()> {
    let api = hidapi::HidApi::new()?;
    println!("VID:PID  usage_page/usage  serial               product");
    for info in api.device_list() {
        if info.usage_page() == 0xFF60 {
            println!(
                "{:04X}:{:04X}  {:04X}/{:04X}       {:<20} {}",
                info.vendor_id(),
                info.product_id(),
                info.usage_page(),
                info.usage(),
                info.serial_number().unwrap_or(""),
                info.product_string().unwrap_or("")
            );
        }
    }
    Ok(())
}

fn ping_cmd(socket: &std::path::Path) -> Result<()> {
    let mut client = Client::connect(socket)?;
    let resp = client.call(&RpcRequest::Ping)?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

fn qsid_list(socket: &std::path::Path) -> Result<()> {
    let mut client = Client::connect(socket)?;
    let resp = client.call(&RpcRequest::QsidList)?;
    let qsids = resp["qsids"].as_array().cloned().unwrap_or_default();
    println!("{:>4}  {:<28}  width", "qsid", "name");
    for e in qsids {
        let q = e["qsid"].as_u64().unwrap_or_default();
        let name = e["name"].as_str().unwrap_or("?");
        let width = e["width"].as_u64().map(|w| format!("{w}B")).unwrap_or_else(|| "?".into());
        println!("{q:>4}  {name:<28}  {width}");
    }
    Ok(())
}

fn qsid_get(socket: &std::path::Path, qsid: u16, width: Option<usize>) -> Result<()> {
    let mut client = Client::connect(socket)?;
    let resp = client.call(&RpcRequest::QsidGet { qsid, width })?;
    println!("{}", resp["value"].as_u64().unwrap_or_default());
    Ok(())
}

fn qsid_set(
    socket: &std::path::Path,
    qsid: u16,
    value: u32,
    width: Option<usize>,
) -> Result<()> {
    let mut client = Client::connect(socket)?;
    let resp = client.call(&RpcRequest::QsidSet { qsid, value, width })?;
    println!("{}", resp["value"].as_u64().unwrap_or_default());
    Ok(())
}

// ----- Helpers duplicated from lib::run() so both the legacy run path
// and the broker path stay in sync. Kept here to avoid rewiring the
// original `run()` API which library-side tests still exercise. -----

fn dedup_key(value: &serde_json::Value) -> serde_json::Value {
    let mut clone = value.clone();
    if let Some(obj) = clone.as_object_mut() {
        obj.remove("reason");
        obj.remove("reason_flags");
    }
    clone
}

fn event_args_from(value: &serde_json::Value) -> Vec<EventArg> {
    const KEYS: &[&str] = &[
        "top_layer_name",
        "default_layer_name",
        "mods_letters",
        "mods_state",
    ];
    let mut out = Vec::with_capacity(KEYS.len());
    if let Some(obj) = value.as_object() {
        for key in KEYS {
            if let Some(v) = obj.get(*key) {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push(EventArg::new(*key, s));
            }
        }
    }
    out
}

// Suppress unused warnings for the "old" run path helpers when only
// broker paths are exercised.
#[allow(dead_code)]
fn _unused_backoff() -> Duration {
    Duration::from_millis(100)
}

#[cfg(test)]
mod tests {
    use super::parse_hex_u16;

    #[test]
    fn parses_hex_prefix() {
        assert_eq!(parse_hex_u16("0x303A").unwrap(), 0x303A);
        assert_eq!(parse_hex_u16("0X4044").unwrap(), 0x4044);
    }

    #[test]
    fn parses_decimal() {
        assert_eq!(parse_hex_u16("12346").unwrap(), 12346);
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(parse_hex_u16("70000").is_err());
        assert!(parse_hex_u16("0xFFFFF").is_err());
    }
}
