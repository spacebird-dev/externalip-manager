use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use externalip_manager_manager::{Manager, ManagerConfig};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Seconds between reconciliation runs, in seconds
    #[arg(short, long, env = "EXTERNALIP_MANAGER_INTERVAL", default_value_t = 60)]
    interval: u32,
    /// Show what actions would be performed without actually modifying any services
    #[arg(long, env = "EXTERNALIP_MANAGER_DRY_RUN", default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let filter_layer = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = fmt::layer()
        .json()
        .with_level(true)
        .with_current_span(false)
        .with_span_list(false)
        .with_target(true);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    if args.dry_run {
        warn!(msg = "Running in dry-run mode, no changes will be made");
    }

    let cfg = ManagerConfig {
        dry_run: args.dry_run,
    };
    let manager = Manager::new(cfg).await?;

    loop {
        match manager.reconcile_svcs().await {
            Ok(errs) if !errs.is_empty() => {
                warn!(
                    msg = "Errors encountered on reconciliation run",
                    errs = ?errs
                );
            }
            Err(e) => {
                error!(msg = "Failed to reconcile resources", err = ?e);
            }
            Ok(_) => {
                info!(msg = "Completed reconciliation");
            }
        };
        tokio::time::sleep(Duration::from_secs(args.interval.into())).await;
    }
}
