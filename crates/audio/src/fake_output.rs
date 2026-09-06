//! An output that plays nothing and lets a test pull the mix on demand.

use std::sync::{Arc, Mutex};

use crate::chunk::{AudioOutput, AudioOutputSession, RenderFn};
use crate::error::AudioError;

#[derive(Default)]
struct Shared {
    render: Mutex<Option<RenderFn>>,
}

pub struct FakeOutput {
    shared: Arc<Shared>,
}

#[derive(Clone)]
pub struct FakeOutputHandle {
    shared: Arc<Shared>,
}

struct Session {
    shared: Arc<Shared>,
}

impl FakeOutput {
    pub fn new() -> (Self, FakeOutputHandle) {
        let shared = Arc::new(Shared::default());
        (
            Self {
                shared: shared.clone(),
            },
            FakeOutputHandle { shared },
        )
    }
}

impl FakeOutputHandle {
    /// Runs the render closure for `frames` stereo frames, as a device callback would.
    pub fn render(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        if let Some(render) = self.shared.render.lock().unwrap().as_mut() {
            render(&mut out);
        }
        out
    }

    pub fn started(&self) -> bool {
        self.shared.render.lock().unwrap().is_some()
    }
}

impl AudioOutput for FakeOutput {
    fn start(&self, render: RenderFn) -> Result<Box<dyn AudioOutputSession>, AudioError> {
        *self.shared.render.lock().unwrap() = Some(render);
        Ok(Box::new(Session {
            shared: self.shared.clone(),
        }))
    }
}

impl AudioOutputSession for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        *self.shared.render.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handle_pulls_the_render_closure_until_the_session_drops() {
        let (output, handle) = FakeOutput::new();
        assert!(!handle.started());
        let session = output
            .start(Box::new(|out: &mut [f32]| out.fill(0.25)))
            .unwrap();
        assert!(handle.started());
        assert_eq!(handle.render(2), vec![0.25; 4]);
        drop(session);
        assert!(!handle.started());
        assert_eq!(handle.render(1), vec![0.0; 2]);
    }
}
