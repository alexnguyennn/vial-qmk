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
use qmk_state_daemon::{run, RunOptions};

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
    /// Read one packet and exit (debug).
    Once(RunArgs),
    /// List raw-HID capable HID devices with usage_page 0xFF60.
    List,
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
        Cmd::Run(args) => run_cmd(args, false),
        Cmd::Once(args) => run_cmd(args, true),
        Cmd::List => list_cmd(),
    }
}

fn run_cmd(args: RunArgs, once: bool) -> Result<()> {
    let mut transport = HidApiTransport::open(args.device.vid, args.device.pid)?;
    let registry = Registry::builder().handler(StateHandler::new()).build();
    let mut writer = FileStateWriter::new(args.state_file);

    let opts = RunOptions {
        sketchybar_event: args.sketchybar_event.clone(),
        once,
        backoff: Some(Duration::from_millis(100)),
        max_backoff: Some(Duration::from_secs(1)),
        max_reads: None,
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
        // clap's default_value_t converts numeric defaults via .to_string(),
        // yielding decimal — parser must accept this.
        assert_eq!(parse_hex_u16("12346").unwrap(), 12346);
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(parse_hex_u16("70000").is_err());
        assert!(parse_hex_u16("0xFFFFF").is_err());
    }
}
