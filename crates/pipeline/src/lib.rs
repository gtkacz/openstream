//! Capture, codec, and transport pipelines.
pub mod fanout;
pub mod reorder;
pub mod slot;
pub use fanout::{FanOut, KeyframeRequest, PushOutcome};
pub use reorder::{Drained, IncomingFrame, Reorder};
pub use slot::{LatestSlot, SlotWait};
