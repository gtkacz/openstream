use crate::CaptureError;
use brp_proto::{PixelFormat, SourceKind};
use std::{future::Future, pin::Pin};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFrame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    pub capture_ts_us: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRequest {
    pub kind: SourceKind,
    pub target_fps: u32,
}
pub type FrameSink = Box<dyn FnMut(CaptureFrame) + Send + 'static>;
pub type StartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn CaptureSession>, CaptureError>> + Send + 'a>>;
pub trait CaptureBackend: Send + Sync {
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_>;
}
pub trait CaptureSession: Send {
    fn info(&self) -> SourceInfo;
    fn stop(self: Box<Self>);
}
