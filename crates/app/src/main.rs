//! brp: peer-to-peer screen sharing.
use std::str::FromStr;

use brp_app::cli::{Cli, Command, WindowArgs};
use brp_app::error::AppError;
use brp_app::launch::Intent;
use brp_app::{participant, publish};
use brp_proto::RoomTicket;
use clap::Parser;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
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
        None => participant::run(&runtime, None, WindowArgs::default()),
        Some(Command::Publish(args)) => runtime.block_on(publish::run(args)),
        Some(Command::Create(args)) => {
            participant::run(&runtime, Some(Intent::Create), args.window)
        }
        Some(Command::Join(args)) => match RoomTicket::from_str(&args.ticket) {
            Ok(ticket) => participant::run(&runtime, Some(Intent::Join(ticket)), args.window),
            Err(error) => Err(AppError::Ticket(error)),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
