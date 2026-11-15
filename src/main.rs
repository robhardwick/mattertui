mod app;
mod config;
mod matter;
mod paths;
mod roots;
mod state;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::matter::Trust;
use crate::paths::Paths;

#[derive(Parser)]
#[command(version, about = "Read-only terminal dashboard for Matter devices")]
struct Cli {
    /// Config file (default: $XDG_CONFIG_HOME/mattertui/config.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Pair a device using the setup code from the Home app
    /// (device → Settings → Turn On Pairing Mode).
    Pair {
        /// Config device id to pair as (a-z, 0-9, -).
        device_id: String,
        /// 11-digit manual code (dashes/spaces allowed) or MT: QR payload.
        code: String,
    },
    /// Remove our fabric from a device and forget it.
    Unpair { device_id: String },
    /// List paired devices.
    Nodes {
        /// Also read each device's endpoints, transport, ICD type and values.
        #[arg(long)]
        probe: bool,
    },
    /// Download production attestation roots (needed before pairing).
    FetchRoots,
    /// Validate the config and show which devices are paired.
    Check,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_stderr_logging();
    let paths = Paths::resolve(cli.config)?;

    match cli.command {
        None => {
            let config = Config::load(&paths.config_file)?;
            app::run(&paths, &config).await
        }
        Some(Command::Check) => check(&paths).await,
        Some(Command::FetchRoots) => {
            tokio::task::spawn_blocking(move || roots::fetch(&paths)).await?
        }
        Some(Command::Pair { device_id, code }) => {
            // Validate before taking the lock or touching the network.
            matter::pairing::validate_device_id(&device_id)?;
            matter::pairing::normalize_code(&code)?;
            let _lock = paths
                .lock()
                .map_err(|e| e.context("press `p` in the dashboard to pair instead"))?;
            let ctl = matter::open(&paths, Trust::Required).await?;
            let info = matter::pairing::pair(&ctl, &device_id, &code).await?;
            println!("paired `{device_id}` as node 0x{:016X}", info.node_id);
            let node = ctl.inner.node(info.node_id);
            print!("{}", matter::probe::probe(&node, &info).await);
            Ok(())
        }
        Some(Command::Unpair { device_id }) => {
            let _lock = paths.lock()?;
            let ctl = matter::open(&paths, Trust::NotNeeded).await?;
            matter::pairing::unpair(&ctl, &device_id).await?;
            println!("unpaired `{device_id}`");
            Ok(())
        }
        Some(Command::Nodes { probe }) => {
            let _lock = paths.lock()?;
            let ctl = matter::open(&paths, Trust::NotNeeded).await?;
            let nodes = ctl.inner.nodes().await?;
            if nodes.is_empty() {
                println!("no paired devices (store: {})", paths.store().display());
            }
            for n in &nodes {
                println!(
                    "{:<24} node 0x{:016X}  vendor {}  product {}",
                    n.label.as_deref().unwrap_or("(no label)"),
                    n.node_id,
                    n.vendor_id.map_or("?".into(), |v| format!("0x{v:04X}")),
                    n.product_id.map_or("?".into(), |v| format!("0x{v:04X}")),
                );
                if probe {
                    print!("{}", matter::probe::probe(&ctl.inner.node(n.node_id), n).await);
                }
            }
            Ok(())
        }
    }
}

async fn check(paths: &Paths) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    println!(
        "{}: {} rooms, {} devices — OK",
        paths.config_file.display(),
        config.rooms.len(),
        config.devices.len()
    );
    let _lock = paths.lock()?;
    let ctl = matter::open(paths, Trust::NotNeeded).await?;
    let nodes = ctl.inner.nodes().await?;
    for room in &config.rooms {
        println!("{}", room.name);
        for dev in config.devices.iter().filter(|d| d.room.get_ref() == room.id.get_ref()) {
            let paired = nodes.iter().any(|n| n.label.as_deref() == Some(dev.id.get_ref()));
            println!(
                "  {:<24} {:<20} {}",
                dev.id.get_ref(),
                format!("{:?}", dev.kind),
                if paired { "paired" } else { "NOT PAIRED" }
            );
        }
    }
    for n in nodes.iter().filter(|n| config.device(n.label.as_deref().unwrap_or("")).is_none()) {
        println!(
            "unassigned: {} (node 0x{:016X}) has no [[device]] entry",
            n.label.as_deref().unwrap_or("(no label)"),
            n.node_id
        );
    }
    Ok(())
}

fn init_stderr_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,mdns_sd=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}
