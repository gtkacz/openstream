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
/// One capturable source on platforms that list them. The value is the platform's raw handle (an
/// `HMONITOR` or `HWND` on Windows) and means nothing across processes or reboots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u64);
/// One entry of a source list: what the picker shows and what `start` is asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub id: SourceId,
    pub kind: SourceKind,
    pub name: String,
    pub width: u32,
    pub height: u32,
}
/// How a platform lets the user choose what to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceListing {
    /// The platform shows its own picker when `start` runs (the Linux portal); nothing to draw.
    PlatformPicker,
    /// The app draws a picker from these and passes the chosen id in `SourceRequest::source`.
    Choices(Vec<SourceDescriptor>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRequest {
    pub kind: SourceKind,
    /// Which listed source to open. Platforms that pick for themselves ignore it.
    pub source: Option<SourceId>,
    pub target_fps: u32,
}
pub type FrameSink = Box<dyn FnMut(CaptureFrame) + Send + 'static>;
pub type StartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn CaptureSession>, CaptureError>> + Send + 'a>>;
pub trait CaptureBackend: Send + Sync {
    /// What `kind` can share. Backends whose platform owns the picker keep the default.
    fn sources(&self, _kind: SourceKind) -> Result<SourceListing, CaptureError> {
        Ok(SourceListing::PlatformPicker)
    }
    fn start(&self, request: SourceRequest, sink: FrameSink) -> StartFuture<'_>;
}
pub trait CaptureSession: Send {
    fn info(&self) -> SourceInfo;
    fn stop(self: Box<Self>);
}
