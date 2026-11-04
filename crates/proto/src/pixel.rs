use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    Bgra,
    Bgrx,
    Rgba,
    Rgbx,
}
impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        4
    }
}
/// How the chroma plane of a decoded or converted frame is sampled.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum ChromaLayout {
    /// One interleaved UV pair per 2x2 luma block: NV12, 4:2:0.
    #[default]
    Half,
    /// One interleaved UV pair per luma sample: NV24, 4:4:4.
    Full,
}
