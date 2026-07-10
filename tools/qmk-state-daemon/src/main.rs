//! qmk-state-daemon CLI entry point.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use qmk_state_daemon::handlers::state::StateHandler;
use qmk_state_daemon::output::FileStateWriter;
use qmk_state_daemon::packet::Registry;
use qmk_state_daemon::sketchybar::{NullNotifier, SketchybarNotifier};
use qmk_state_daemon::transport::HidApiTransport;
use qmk_state_daemon::{run, send_query, RunOptions};

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
    /// Run the daemon: read packets, write JSON, trigger sketchybar.
    Run(RunArgs),
    /// Send a single query request to the keyboard and exit.
    Query(DeviceArgs),
    /// Read one packet and exit (debug).
    Once(RunArgs),
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

    /// Skip the initial query on connect.
    #[arg(long)]
    no_query_on_start: bool,
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u16::from_str_radix(stripped, 16).map_err(|e| e.to_string())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run(args) => run_cmd(args, false),
        Cmd::Once(args) => run_cmd(args, true),
        Cmd::Query(dev) => {
            let mut t = HidApiTransport::open(dev.vid, dev.pid)?;
            send_query(&mut t)?;
            Ok(())
        }
    }
}

fn run_cmd(args: RunArgs, once: bool) -> Result<()> {
    let mut transport = HidApiTransport::open(args.device.vid, args.device.pid)?;
    let registry = Registry::builder().handler(StateHandler::new()).build();
    let mut writer = FileStateWriter::new(args.state_file);

    let opts = RunOptions {
        sketchybar_event: args.sketchybar_event.clone(),
        query_on_start: !args.no_query_on_start,
        once,
        backoff: Some(Duration::from_millis(100)),
        max_backoff: Some(Duration::from_secs(1)),
    };

    if args.dry_run {
        run(
            &mut transport,
            &registry,
            &mut writer,
            &NullNotifier,
            &opts,
        )
    } else {
        run(
            &mut transport,
            &registry,
            &mut writer,
            &SketchybarNotifier,
            &opts,
        )
    }
}
