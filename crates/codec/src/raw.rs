use std::sync::Mutex;

use crate::CodecError;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub y_stride: usize,
    pub uv_stride: usize,
    pub y: Vec<u8>,
    pub uv: Vec<u8>,
    pub capture_ts_us: u64,
}
impl RawFrame {
    pub fn black(w: u32, h: u32, ts: u64) -> Self {
        let s = w as usize;
        Self {
            width: w,
            height: h,
            y_stride: s,
            uv_stride: s,
            y: vec![16; s * h as usize],
            uv: vec![128; s * (h as usize).div_ceil(2)],
            capture_ts_us: ts,
        }
    }
    pub fn chroma_rows(&self) -> usize {
        (self.height as usize).div_ceil(2)
    }
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return Err(CodecError::InvalidFrame(
                "dimensions must be even and non-zero".into(),
            ));
        }
        if self.y_stride < self.width as usize || self.uv_stride < self.width as usize {
            return Err(CodecError::InvalidFrame("stride shorter than width".into()));
        }
        if self.y.len() < self.y_stride * self.height as usize
            || self.uv.len() < self.uv_stride * self.chroma_rows()
        {
            return Err(CodecError::InvalidFrame(
                "buffer shorter than stride".into(),
            ));
        }
        Ok(())
    }
}

/// A bounded free list of previously used frame buffers, shared (via `Arc`) between the decode
/// thread that acquires buffers and whichever thread releases them once nothing reads them any
/// more — a frame superseded before display, or a displayed frame once its GPU upload is queued.
/// Reusing a buffer skips both the allocation and the black-level initialization
/// `RawFrame::black` performs on a buffer that is about to be fully overwritten. Buffers are
/// matched by exact `(width, height)`; a stride, chroma layout, or resolution change never reuses
/// a mismatched buffer, so it falls back to a fresh `RawFrame::black`. Bounded by `capacity` so a
/// stalled consumer cannot grow retained memory without limit; dropping the pool (e.g. when a
/// watch ends) releases everything it holds.
pub struct RawFramePool {
    capacity: usize,
    free: Mutex<Vec<RawFrame>>,
}

impl RawFramePool {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            free: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<RawFrame>> {
        self.free.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Reuses a pooled buffer matching `width`/`height` when one is free; otherwise allocates a
    /// fresh black frame.
    pub fn acquire(&self, width: u32, height: u32, capture_ts_us: u64) -> RawFrame {
        let mut free = self.lock();
        if let Some(pos) = free
            .iter()
            .position(|f| f.width == width && f.height == height)
        {
            let mut frame = free.swap_remove(pos);
            frame.capture_ts_us = capture_ts_us;
            frame
        } else {
            // Evict stale-shape buffers so a resolution/preset change doesn't permanently pin
            // the free list full of buffers `acquire` can never match and `release` can never
            // replace (it only accepts new buffers while `free.len() < capacity`).
            free.retain(|f| f.width == width && f.height == height);
            drop(free);
            RawFrame::black(width, height, capture_ts_us)
        }
    }

    /// Gives a frame no consumer will read back to the pool, subject to `capacity`.
    pub fn release(&self, frame: RawFrame) {
        let mut free = self.lock();
        if free.len() < self.capacity {
            free.push(frame);
        }
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_has_limited_range_black_and_tight_strides() {
        let f = RawFrame::black(6, 4, 77);
        assert_eq!(
            (f.y_stride, f.uv_stride, f.y.len(), f.uv.len()),
            (6, 6, 24, 12)
        );
        assert!(f.y.iter().all(|&v| v == 16) && f.uv.iter().all(|&v| v == 128));
        assert!(f.validate().is_ok());
    }

    #[test]
    fn validate_rejects_short_buffers_and_odd_sizes() {
        let mut f = RawFrame::black(6, 4, 0);
        f.y.pop();
        assert!(matches!(f.validate(), Err(CodecError::InvalidFrame(_))));
        let odd = RawFrame {
            width: 5,
            ..RawFrame::black(6, 4, 0)
        };
        assert!(matches!(odd.validate(), Err(CodecError::InvalidFrame(_))));
    }

    #[test]
    fn pool_reuses_a_released_buffer_of_the_same_shape() {
        let pool = RawFramePool::new(2);
        let a = pool.acquire(6, 4, 1);
        let y_ptr = a.y.as_ptr();
        let uv_ptr = a.uv.as_ptr();
        pool.release(a);
        let b = pool.acquire(6, 4, 2);
        assert_eq!(
            b.y.as_ptr(),
            y_ptr,
            "a matching shape must reuse the y allocation"
        );
        assert_eq!(
            b.uv.as_ptr(),
            uv_ptr,
            "a matching shape must reuse the uv allocation"
        );
        assert_eq!(b.capture_ts_us, 2);
        assert!(pool.is_empty(), "the reused buffer leaves the pool empty");
    }

    #[test]
    fn pool_drops_buffers_beyond_its_bound() {
        let pool = RawFramePool::new(1);
        pool.release(RawFrame::black(6, 4, 0));
        pool.release(RawFrame::black(6, 4, 0));
        assert_eq!(
            pool.len(),
            1,
            "a pool must never retain more than its bound"
        );
    }

    #[test]
    fn pool_never_reuses_a_mismatched_shape() {
        let pool = RawFramePool::new(2);
        pool.release(RawFrame::black(6, 4, 0));
        // Different width and height: a resolution/chroma-layout change, not a stable stream.
        let out = pool.acquire(10, 6, 5);
        assert_eq!((out.width, out.height, out.capture_ts_us), (10, 6, 5));
        out.validate().unwrap();
        assert_eq!(
            pool.len(),
            0,
            "a mismatched buffer is evicted, not retained, on a shape-mismatch miss"
        );
    }

    #[test]
    fn pool_resumes_reuse_at_the_new_shape_after_a_resolution_change() {
        let pool = RawFramePool::new(2);
        pool.release(RawFrame::black(6, 4, 0));
        // The mismatch eviction above must not disable pooling permanently: once a
        // new-shape buffer is released, later same-shape acquisitions reuse it.
        let miss = pool.acquire(10, 6, 1);
        pool.release(miss);
        let a = pool.acquire(10, 6, 2);
        let y_ptr = a.y.as_ptr();
        pool.release(a);
        let b = pool.acquire(10, 6, 3);
        assert_eq!(
            b.y.as_ptr(),
            y_ptr,
            "pooling resumes reusing allocations at the new shape"
        );
    }

    #[test]
    fn pool_falls_back_to_black_when_empty() {
        let pool = RawFramePool::new(2);
        let out = pool.acquire(6, 4, 9);
        assert!(out.y.iter().all(|&v| v == 16) && out.uv.iter().all(|&v| v == 128));
        assert!(out.validate().is_ok());
    }
}
