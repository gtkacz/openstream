use std::ffi::c_int;
use std::ptr;

use brp_proto::PixelFormat;
use ffmpeg_sys_next as ff;

use crate::error::CodecError;
use crate::ffmpeg::ffi::{check, init_logging};
use crate::raw::RawFrame;
use crate::traits::{FrameConverter, InputImage};

/// Converts packed 32-bit RGB into limited-range BT.709 NV12 at the destination size.
pub struct SwsConverter {
    ctx: *mut ff::SwsContext,
    src: (u32, u32, PixelFormat),
    dst_width: u32,
    dst_height: u32,
}

fn av_pix_fmt(format: PixelFormat) -> ff::AVPixelFormat {
    match format {
        PixelFormat::Bgra => ff::AVPixelFormat::AV_PIX_FMT_BGRA,
        PixelFormat::Bgrx => ff::AVPixelFormat::AV_PIX_FMT_BGR0,
        PixelFormat::Rgba => ff::AVPixelFormat::AV_PIX_FMT_RGBA,
        PixelFormat::Rgbx => ff::AVPixelFormat::AV_PIX_FMT_RGB0,
    }
}

impl SwsConverter {
    pub fn new(
        src_width: u32,
        src_height: u32,
        src_format: PixelFormat,
        dst_width: u32,
        dst_height: u32,
    ) -> Result<Self, CodecError> {
        init_logging();
        if src_width == 0
            || src_height == 0
            || dst_width == 0
            || dst_height == 0
            || !dst_width.is_multiple_of(2)
            || !dst_height.is_multiple_of(2)
        {
            return Err(CodecError::InvalidFrame(format!(
                "invalid conversion dimensions {src_width}x{src_height} -> {dst_width}x{dst_height}"
            )));
        }
        let ctx = unsafe {
            ff::sws_getContext(
                src_width as c_int,
                src_height as c_int,
                av_pix_fmt(src_format),
                dst_width as c_int,
                dst_height as c_int,
                ff::AVPixelFormat::AV_PIX_FMT_NV12,
                ff::SwsFlags::SWS_BILINEAR as c_int,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if ctx.is_null() {
            return Err(CodecError::InvalidFrame(
                "swscale could not create a conversion context".into(),
            ));
        }
        let result = unsafe {
            let coeffs = ff::sws_getCoefficients(ff::SWS_CS_ITU709 as c_int);
            check(
                "sws_setColorspaceDetails",
                ff::sws_setColorspaceDetails(ctx, coeffs, 1, coeffs, 0, 0, 1 << 16, 1 << 16),
            )
        };
        if let Err(error) = result {
            unsafe { ff::sws_freeContext(ctx) };
            return Err(error);
        }
        Ok(Self {
            ctx,
            src: (src_width, src_height, src_format),
            dst_width,
            dst_height,
        })
    }
}

impl FrameConverter for SwsConverter {
    fn convert(&mut self, src: &InputImage<'_>) -> Result<RawFrame, CodecError> {
        if src.width == 0
            || src.height == 0
            || src.stride < src.width as usize * src.format.bytes_per_pixel()
        {
            return Err(CodecError::InvalidFrame(
                "input stride is shorter than one row".into(),
            ));
        }
        let needed = src
            .stride
            .checked_mul(src.height as usize)
            .ok_or_else(|| CodecError::InvalidFrame("input dimensions overflow usize".into()))?;
        if src.data.len() < needed {
            return Err(CodecError::InvalidFrame(format!(
                "input holds {} bytes but needs {needed}",
                src.data.len()
            )));
        }
        if self.src != (src.width, src.height, src.format) {
            *self = Self::new(
                src.width,
                src.height,
                src.format,
                self.dst_width,
                self.dst_height,
            )?;
        }
        let mut out = RawFrame::black(self.dst_width, self.dst_height, src.capture_ts_us);
        let source = [src.data.as_ptr(), ptr::null(), ptr::null(), ptr::null()];
        let source_stride = [src.stride as c_int, 0, 0, 0];
        let destination = [
            out.y.as_mut_ptr(),
            out.uv.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
        ];
        let destination_stride = [out.y_stride as c_int, out.uv_stride as c_int, 0, 0];
        let rows = unsafe {
            ff::sws_scale(
                self.ctx,
                source.as_ptr(),
                source_stride.as_ptr(),
                0,
                src.height as c_int,
                destination.as_ptr(),
                destination_stride.as_ptr(),
            )
        };
        check("sws_scale", rows)?;
        Ok(out)
    }
}

impl Drop for SwsConverter {
    fn drop(&mut self) {
        unsafe { ff::sws_freeContext(self.ctx) }
    }
}
unsafe impl Send for SwsConverter {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{FrameConverter, InputImage};

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> Vec<u8> {
        bgra.iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect()
    }
    fn convert(width: u32, height: u32, pixels: &[u8], dst: (u32, u32)) -> RawFrame {
        let mut converter =
            SwsConverter::new(width, height, PixelFormat::Bgra, dst.0, dst.1).unwrap();
        let image = InputImage {
            width,
            height,
            stride: (width * 4) as usize,
            format: PixelFormat::Bgra,
            data: pixels,
            capture_ts_us: 42,
        };
        converter.convert(&image).unwrap()
    }
    #[test]
    fn white_maps_to_limited_range_white() {
        let output = convert(8, 4, &solid(8, 4, [255; 4]), (8, 4));
        assert_eq!(
            (output.width, output.height, output.capture_ts_us),
            (8, 4, 42)
        );
        assert!(output.y.iter().all(|&v| (233..=237).contains(&v)));
        assert!(output.uv.iter().all(|&v| (126..=130).contains(&v)));
    }
    #[test]
    fn black_maps_to_limited_range_black_and_scales() {
        let output = convert(16, 8, &solid(16, 8, [0, 0, 0, 255]), (8, 4));
        assert_eq!((output.width, output.height), (8, 4));
        assert_eq!(output.y.len(), 32);
        assert_eq!(output.uv.len(), 16);
        assert!(output.y.iter().all(|&v| (14..=18).contains(&v)));
        assert!(output.validate().is_ok());
    }
    #[test]
    fn short_input_buffer_is_rejected() {
        let mut converter = SwsConverter::new(8, 4, PixelFormat::Bgra, 8, 4).unwrap();
        let image = InputImage {
            width: 8,
            height: 4,
            stride: 32,
            format: PixelFormat::Bgra,
            data: &[0; 10],
            capture_ts_us: 0,
        };
        assert!(matches!(
            converter.convert(&image),
            Err(CodecError::InvalidFrame(_))
        ));
    }
}
