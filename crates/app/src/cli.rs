use brp_proto::{Codec, SourceKind};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "brp", about = "Peer-to-peer screen sharing", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    Publish(PublishArgs),
    Watch(WatchArgs),
}
#[derive(Args, Debug)]
pub struct PublishArgs {
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    #[arg(long)]
    pub bitrate_kbps: Option<u32>,
    #[arg(long, value_enum)]
    pub codec: Option<CodecArg>,
    #[arg(long, value_enum, default_value_t = SourceArg::Monitor)]
    pub source: SourceArg,
    #[arg(long)]
    pub no_relay: bool,
    /// Join this room instead of creating a new one.
    #[arg(long)]
    pub ticket: Option<String>,
    /// Shown to other participants. Defaults to the short peer id.
    #[arg(long)]
    pub nickname: Option<String>,
}
#[derive(Args, Debug)]
pub struct WatchArgs {
    pub ticket: String,
    #[arg(long)]
    pub no_relay: bool,
    #[arg(long)]
    pub nickname: Option<String>,
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
