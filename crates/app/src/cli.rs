use brp_proto::{Codec, SourceKind};
use clap::{Args, Parser, Subcommand, ValueEnum};

/// Capture ceiling when no `--fps` is given, and the start screen's default.
pub const DEFAULT_FPS: u32 = 60;

#[derive(Parser, Debug)]
#[command(name = "brp", about = "Peer-to-peer screen sharing", version)]
pub struct Cli {
    /// Without a subcommand the participant window opens on its start screen.
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Share one live headlessly and print the ticket.
    Publish(PublishArgs),
    /// Open a new room in the participant window.
    Create(CreateArgs),
    /// Join a room with a ticket or link in the participant window.
    Join(JoinArgs),
}
#[derive(Args, Debug)]
pub struct PublishArgs {
    #[arg(long, default_value_t = DEFAULT_FPS)]
    pub fps: u32,
    #[arg(long)]
    pub bitrate_kbps: Option<u32>,
    #[arg(long, value_enum)]
    pub codec: Option<CodecArg>,
    #[arg(long, value_enum, default_value_t = SourceArg::Monitor)]
    pub source: SourceArg,
    #[arg(long)]
    pub no_relay: bool,
    /// Join this room, given as a ticket or link, instead of creating a new one.
    #[arg(long)]
    pub ticket: Option<String>,
    /// Shown to other participants. Defaults to the short peer id.
    #[arg(long)]
    pub nickname: Option<String>,
}
#[derive(Args, Debug, Default, Clone)]
pub struct WindowArgs {
    /// Shown to other participants. Overrides the saved nickname for this launch.
    #[arg(long)]
    pub nickname: Option<String>,
    /// Capture ceiling for lives shared from the window; each live's presets can go lower.
    /// Overrides the saved setting for this launch.
    #[arg(long)]
    pub fps: Option<u32>,
    /// Disables relays for this launch whatever the saved relay setting is.
    #[arg(long)]
    pub no_relay: bool,
}
#[derive(Args, Debug)]
pub struct CreateArgs {
    #[command(flatten)]
    pub window: WindowArgs,
}
#[derive(Args, Debug)]
pub struct JoinArgs {
    /// A ticket, a share link, or a brp:// link.
    pub ticket: String,
    #[command(flatten)]
    pub window: WindowArgs,
}
#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum CodecArg {
    H264,
    Hevc,
    Av1,
}
impl From<CodecArg> for Codec {
    fn from(c: CodecArg) -> Self {
        match c {
            CodecArg::H264 => Self::H264,
            CodecArg::Hevc => Self::Hevc,
            CodecArg::Av1 => Self::Av1,
        }
    }
}
#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum SourceArg {
    Monitor,
    Window,
}
impl From<SourceArg> for SourceKind {
    fn from(s: SourceArg) -> Self {
        match s {
            SourceArg::Monitor => Self::Monitor,
            SourceArg::Window => Self::Window,
        }
    }
}
