//! brp: peer-to-peer screen sharing.
mod cli;
mod error;
mod identity;
mod publish;
mod render;
mod watch;
mod window;

use crate::cli::{Cli, Command};
use clap::Parser;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            // zbus warns when the portal omits optional properties; that is expected and not actionable.
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,zbus=error")),
        )
        .init();
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: could not start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cli.command {
        Command::Publish(args) => runtime.block_on(publish::run(args)),
        Command::Watch(args) => watch::run(&runtime, args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
