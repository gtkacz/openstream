//! What the media server serves from: lives and audio from the registry, voice from the controller.

use std::sync::Arc;

use brp_net::{AudioSubscription, LiveSource, SubscribeRejected, Subscription};
use iroh::EndpointId;

use crate::registry::LiveRegistry;
use crate::voice::Voice;

pub(crate) struct MediaSource {
    pub lives: Arc<LiveRegistry>,
    pub voice: Arc<Voice>,
}

impl LiveSource for MediaSource {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        self.lives.subscribe(live_id, preset_id)
    }

    fn request_keyframe(&self, live_id: u32, preset_id: u32) {
        self.lives.request_keyframe(live_id, preset_id);
    }

    fn subscribe_audio(&self, live_id: u32) -> Result<AudioSubscription, SubscribeRejected> {
        self.lives.subscribe_audio(live_id)
    }

    fn subscribe_voice(&self, peer: EndpointId) -> Result<AudioSubscription, SubscribeRejected> {
        self.voice.subscribe(peer)
    }
}
