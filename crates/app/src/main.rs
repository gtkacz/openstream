#![cfg_attr(windows, windows_subsystem = "windows")]
//! brp: peer-to-peer screen sharing.
use brp_app::cli::{Cli, Command, WindowArgs};
use brp_app::error::AppError;
use brp_app::launch::Intent;
use brp_app::link;
use brp_app::{participant, publish};
use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    brp_app::console::attach_parent_console();
    let log = brp_app::logging::init();
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
        None => participant::run(&runtime, None, None, WindowArgs::default()),
        Some(Command::Publish(args)) => runtime.block_on(publish::run(args)),
        Some(Command::Create(args)) => {
            participant::run(&runtime, Some(Intent::Create), None, args.window)
        }
        Some(Command::Join(args)) => match link::parse_ticket(&args.ticket) {
            Ok(ticket) => participant::run(&runtime, Some(Intent::Join(ticket)), None, args.window),
            // A browser launches the binary without a console, so the error must reach the
            // window, not stderr.
            Err(error) => participant::run(
                &runtime,
                None,
                Some(AppError::Ticket(error).to_string()),
                args.window,
            ),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            if let Some(log) = log {
                eprintln!("log: {}", log.display());
            }
            ExitCode::FAILURE
        }
    }
}
