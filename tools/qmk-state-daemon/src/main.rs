//! qmk-state-daemon CLI entry point.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use qmk_state_daemon::config::{
    default_config_path, default_config_text, default_socket_path, Config, SinkConfig,
    DEFAULT_EVENT_NAME,
};
use qmk_state_daemon::handlers::state::StateHandler;
use qmk_state_daemon::output::{FileStateWriter, StateWriter};
use qmk_state_daemon::packet::Registry;
use qmk_state_daemon::rpc::{self, Client, RpcRequest, Server};
use qmk_state_daemon::sink::{EventArg, EventSink};
#[cfg(target_os = "macos")]
use qmk_state_daemon::sinks::SketchybarSink;
use qmk_state_daemon::sinks::{CommandSink, NullSink};
use qmk_state_daemon::transport::HidApiTransport;
use qmk_state_daemon::vial_qsid;

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
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Reload sink/event/state-file settings from the daemon config.
    ReloadConfig {
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Print the default config path.
    ConfigPath,
    /// Write a default config file.
    WriteDefaultConfig {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum QsidCmd {
    /// List all supported QSIDs on the connected keyboard.
    List {
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Read a QSID value.
    Get {
        qsid: u16,
        #[arg(long, value_parser = ["1", "2", "4"])]
        width: Option<String>,
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Write a QSID value; prints the read-back value on success.
    Set {
        qsid: u16,
        value: u32,
        #[arg(long, value_parser = ["1", "2", "4"])]
        width: Option<String>,
        #[arg(long)]
        socket: Option<PathBuf>,
    },
}

#[derive(Args, Clone)]
struct DeviceArgs {
    #[arg(long, value_parser = parse_hex_u16)]
    vid: Option<u16>,
    #[arg(long, value_parser = parse_hex_u16)]
    pid: Option<u16>,
}

#[derive(Args, Clone)]
struct RunArgs {
    #[command(flatten)]
    device: DeviceArgs,

    /// Config file path (defaults to XDG config location).
    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long)]
    state_file: Option<PathBuf>,

    /// Event name passed to the configured sink. Hidden old alias is
    /// kept for existing launchd plists/scripts.
    #[arg(long, alias = "sketchybar-event", default_value = DEFAULT_EVENT_NAME)]
    event_name: String,

    /// Runtime-selected sink. Overrides config when supplied.
    #[arg(long, value_parser = ["sketchybar", "command", "null"])]
    sink: Option<String>,

    /// Program argv for command sink. Repeat the flag for arguments:
    /// `--command /path/script --command arg1`.
    #[arg(long)]
    command: Vec<String>,

    /// Log instead of invoking sketchybar.
    #[arg(long)]
    dry_run: bool,

    /// Path of the local RPC socket used by `qsid`/`ping` subcommands.
    #[arg(long)]
    socket: Option<PathBuf>,
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
        Cmd::Ping { socket } => ping_cmd(&socket.unwrap_or_else(default_socket_path)),
        Cmd::ReloadConfig { socket } => {
            reload_config_cmd(&socket.unwrap_or_else(default_socket_path))
        }
        Cmd::ConfigPath => {
            println!("{}", default_config_path().display());
            Ok(())
        }
        Cmd::WriteDefaultConfig { force, path } => write_default_config(path, force),
        Cmd::Qsid(qs) => match qs {
            QsidCmd::List { socket } => qsid_list(&socket.unwrap_or_else(default_socket_path)),
            QsidCmd::Get {
                qsid,
                width,
                socket,
            } => qsid_get(
                &socket.unwrap_or_else(default_socket_path),
                qsid,
                parse_width(&width)?,
            ),
            QsidCmd::Set {
                qsid,
                value,
                width,
                socket,
            } => qsid_set(
                &socket.unwrap_or_else(default_socket_path),
                qsid,
                value,
                parse_width(&width)?,
            ),
        },
    }
}

fn parse_width(w: &Option<String>) -> Result<Option<usize>> {
    Ok(w.as_deref().map(|s| s.parse::<usize>().unwrap()))
}

fn run_cmd(args: RunArgs) -> Result<()> {
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    }
    .resolve()?;

    let vid = args.device.vid.unwrap_or(config.vid);
    let pid = args.device.pid.unwrap_or(config.pid);
    let state_file = args.state_file.clone().unwrap_or(config.state_file);
    let socket = args.socket.clone().unwrap_or(config.socket);
    let sink_cfg = sink_config_from_args(&args, config.sink)?;
    let event = event_name_for(&sink_cfg, &args.event_name);

    let transport = HidApiTransport::open(vid, pid)?;
    let registry = Registry::builder().handler(StateHandler::new()).build();
    let runtime = Arc::new(Mutex::new(RuntimeState {
        event,
        state_file: state_file.clone(),
        writer: Box::new(FileStateWriter::new(state_file.clone())),
        sink: make_sink(&sink_cfg, &state_file, args.dry_run)?,
        dry_run: args.dry_run,
    }));
    let mut last_key: Option<serde_json::Value> = None;

    // State packet handler: decode, dedup, write JSON, fire notifier.
    let state_runtime = runtime.clone();
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
        let mut rt = state_runtime.lock().unwrap();
        if let Err(e) = rt.writer.write(&value) {
            eprintln!("write state file error: {e}");
            return;
        }
        let ev_args = event_args_from(&value);
        let payload_json = match serde_json::to_string(&value) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("serialize state payload error: {e}");
                return;
            }
        };
        if let Err(e) = rt.sink.emit(&rt.event, &ev_args, &payload_json) {
            eprintln!("sink error: {e}");
        }
    };

    let (broker, _broker_handle) = rpc::spawn_broker(transport, on_state);

    // Serve socket in current thread.
    let reload_config_path = config_path.clone();
    let reload_event_default = args.event_name.clone();
    let reload_runtime = runtime.clone();
    let reload_dry_run = args.dry_run;
    let server = Server::bind(&socket, broker)
        .context("bind rpc socket")?
        .with_reload_handler(move || {
            let cfg = if reload_config_path.exists() {
                Config::load(&reload_config_path)?
            } else {
                Config::default()
            }
            .resolve()?;
            let sink_cfg = cfg.sink;
            let event = event_name_for(&sink_cfg, &reload_event_default);
            let state_file = cfg.state_file;
            let sink = make_sink(&sink_cfg, &state_file, reload_dry_run)?;
            let mut rt = reload_runtime.lock().unwrap();
            rt.event = event.clone();
            rt.state_file = state_file.clone();
            rt.writer = Box::new(FileStateWriter::new(state_file.clone()));
            rt.sink = sink;
            Ok(serde_json::json!({
                "ok": true,
                "event": event,
                "state_file": state_file,
                "restart_required_for": ["vid", "pid", "socket"]
            }))
        });
    println!(
        "qmk-state-daemon running: rpc socket = {}",
        socket.display()
    );
    server.serve()?;
    Ok(())
}

struct RuntimeState {
    event: String,
    state_file: PathBuf,
    writer: Box<dyn StateWriter>,
    sink: Box<dyn EventSink>,
    #[allow(dead_code)]
    dry_run: bool,
}

fn write_default_config(path: Option<PathBuf>, force: bool) -> Result<()> {
    let path = path.unwrap_or_else(default_config_path);
    if path.exists() && !force {
        anyhow::bail!("{} exists; pass --force to overwrite", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, default_config_text())?;
    println!("wrote {}", path.display());
    Ok(())
}

fn sink_config_from_args(args: &RunArgs, fallback: SinkConfig) -> Result<SinkConfig> {
    if args.dry_run {
        return Ok(SinkConfig::Null {
            event: Some(args.event_name.clone()),
        });
    }
    match args.sink.as_deref() {
        None => {
            if args.command.is_empty() {
                Ok(fallback)
            } else {
                Ok(SinkConfig::Command {
                    event: Some(args.event_name.clone()),
                    program: args.command.clone(),
                })
            }
        }
        Some("null") => Ok(SinkConfig::Null {
            event: Some(args.event_name.clone()),
        }),
        Some("command") => Ok(SinkConfig::Command {
            event: Some(args.event_name.clone()),
            program: args.command.clone(),
        }),
        Some("sketchybar") => Ok(SinkConfig::Sketchybar {
            event: Some(args.event_name.clone()),
        }),
        Some(other) => anyhow::bail!("unknown sink {other}"),
    }
}

fn event_name_for(cfg: &SinkConfig, cli_default: &str) -> String {
    match cfg {
        SinkConfig::Sketchybar { event }
        | SinkConfig::Command { event, .. }
        | SinkConfig::Null { event } => event.clone().unwrap_or_else(|| cli_default.to_string()),
    }
}

fn make_sink(
    cfg: &SinkConfig,
    state_file: &std::path::Path,
    dry_run: bool,
) -> Result<Box<dyn EventSink>> {
    if dry_run {
        return Ok(Box::new(NullSink));
    }
    match cfg {
        SinkConfig::Null { .. } => Ok(Box::new(NullSink)),
        SinkConfig::Command { program, .. } => Ok(Box::new(CommandSink::new(
            program.clone(),
            state_file.to_path_buf(),
        )?)),
        SinkConfig::Sketchybar { .. } => make_sketchybar_sink(),
    }
}

#[cfg(target_os = "macos")]
fn make_sketchybar_sink() -> Result<Box<dyn EventSink>> {
    Ok(Box::new(SketchybarSink))
}

#[cfg(not(target_os = "macos"))]
fn make_sketchybar_sink() -> Result<Box<dyn EventSink>> {
    anyhow::bail!("sketchybar sink is only available on macOS")
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

fn reload_config_cmd(socket: &std::path::Path) -> Result<()> {
    let mut client = Client::connect(socket)?;
    let resp = client.call(&RpcRequest::ReloadConfig)?;
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
        let width = e["width"]
            .as_u64()
            .map(|w| format!("{w}B"))
            .unwrap_or_else(|| "?".into());
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

fn qsid_set(socket: &std::path::Path, qsid: u16, value: u32, width: Option<usize>) -> Result<()> {
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
