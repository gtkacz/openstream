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
        None => participant::run(&runtime, None, None, None, WindowArgs::default()),
        Some(Command::Publish(args)) => runtime.block_on(publish::run(args)),
        Some(Command::Create(args)) => {
            participant::run(&runtime, Some(Intent::Create), None, None, args.window)
        }
        Some(Command::Join(args)) => match link::parse_ticket(&args.ticket) {
            // A scheme link can come from any web page. A ticket typed here is the person's own
            // choice, and so is the relaunch after an update, which rejoins with a bare ticket.
            Ok(ticket) if link::is_scheme_link(&args.ticket) => {
                participant::run(&runtime, None, Some(ticket), None, args.window)
            }
            Ok(ticket) => participant::run(
                &runtime,
                Some(Intent::Join(ticket)),
                None,
                None,
                args.window,
            ),
            // Both destinations are needed: a browser launches the binary without a console, so
            // the window has to carry the message, and a terminal launch on a machine that cannot
            // open a window would otherwise lose the diagnosis.
            Err(error) => {
                let message = AppError::Ticket(error).to_string();
                eprintln!("error: {message}");
                participant::run(&runtime, None, None, Some(message), args.window)
            }
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
