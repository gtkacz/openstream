//! The listening half of voice: one subscription per person in voice, decoded into the voice mixer.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use brp_net::ReceivedFrame;
use brp_pipeline::{AudioViewer, AudioViewerStats, Mixer};
use brp_proto::constants::{
    AUDIO_SENDER_BACKLOG_PACKETS, RESUBSCRIBE_BACKOFF_INITIAL, RESUBSCRIBE_BACKOFF_MAX,
};
use iroh::PublicKey;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;

use crate::codecs::DecoderFactory;
use crate::error::RoomError;
use crate::gossip::lock;
use crate::watcher::Watcher;

/// Counts every packet, but passes it on only while `hearing` is set. Muting someone must keep the
/// subscription up and the received-bytes count moving, while the decoder sees nothing.
pub(crate) async fn forward_heard(
    mut packets: Receiver<ReceivedFrame>,
    out: Sender<ReceivedFrame>,
    hearing: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
) {
    while let Some(packet) = packets.recv().await {
        bytes.fetch_add(packet.payload.len() as u64, Ordering::Relaxed);
        if hearing.load(Ordering::Relaxed) && out.send(packet).await.is_err() {
            return;
        }
    }
}

pub(crate) struct Listener {
    peer: PublicKey,
    mixer: Mixer,
    hearing: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    // Shared with the task so `stop` can end the decode thread the aborted task would leave behind.
    viewer: Arc<Mutex<Option<AudioViewer>>>,
    task: JoinHandle<()>,
}

impl Listener {
    pub(crate) fn start(
        peer: PublicKey,
        watcher: Arc<Watcher>,
        decoders: Arc<dyn DecoderFactory>,
        mixer: Mixer,
        hearing: bool,
        runtime: Handle,
    ) -> Self {
        let hearing = Arc::new(AtomicBool::new(hearing));
        let bytes = Arc::new(AtomicU64::new(0));
        let viewer = Arc::new(Mutex::new(None));
        let task = runtime.spawn(keep_listening(
            peer,
            watcher,
            decoders,
            mixer.clone(),
            hearing.clone(),
            bytes.clone(),
            viewer.clone(),
            runtime.clone(),
        ));
        Self {
            peer,
            mixer,
            hearing,
            bytes,
            viewer,
            task,
        }
    }

    pub(crate) fn set_hearing(&self, hearing: bool) {
        self.hearing.store(hearing, Ordering::Relaxed);
    }

    pub(crate) fn bytes_received(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub(crate) fn stop(self) {
        self.task.abort();
        if let Some(viewer) = lock(&self.viewer).take() {
            viewer.stop();
        }
        self.mixer.remove_track(self.peer.as_bytes());
    }
}

#[allow(clippy::too_many_arguments)]
async fn keep_listening(
    peer: PublicKey,
    watcher: Arc<Watcher>,
    decoders: Arc<dyn DecoderFactory>,
    mixer: Mixer,
    hearing: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    slot: Arc<Mutex<Option<AudioViewer>>>,
    runtime: Handle,
) {
    let mut backoff = RESUBSCRIBE_BACKOFF_INITIAL;
    loop {
        match listen_once(
            peer, &watcher, &decoders, &mixer, &hearing, &bytes, &slot, &runtime,
        )
        .await
        {
            Ok(()) => backoff = RESUBSCRIBE_BACKOFF_INITIAL,
            Err(error) => tracing::debug!(%error, "could not hear a member in voice yet"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
    }
}

/// Holds one subscription until its stream ends. The control sender stays alive for the whole
/// round and is dropped at its end, which frees the connection's single voice slot for the next.
#[allow(clippy::too_many_arguments)]
async fn listen_once(
    peer: PublicKey,
    watcher: &Watcher,
    decoders: &Arc<dyn DecoderFactory>,
    mixer: &Mixer,
    hearing: &Arc<AtomicBool>,
    bytes: &Arc<AtomicU64>,
    slot: &Mutex<Option<AudioViewer>>,
    runtime: &Handle,
) -> Result<(), RoomError> {
    let client = watcher.client_for(peer).await?;
    let subscription = client.subscribe_voice().await?;
    let decoder = decoders.open_audio(&subscription.params)?;
    let (tx, rx) = mpsc::channel(AUDIO_SENDER_BACKLOG_PACKETS);
    let forwarder = runtime.spawn(forward_heard(
        subscription.packets,
        tx,
        hearing.clone(),
        bytes.clone(),
    ));
    let mut viewer = AudioViewer::start(
        runtime.clone(),
        rx,
        decoder,
        mixer.add_track(*peer.as_bytes()),
        Arc::new(AudioViewerStats::default()),
    );
    let mut ended = viewer.take_ended();
    *lock(slot) = Some(viewer);
    if let Some(ended) = ended.as_mut() {
        ended.recv().await;
    }
    forwarder.abort();
    let viewer = lock(slot).take();
    if let Some(viewer) = viewer {
        let _ = tokio::task::spawn_blocking(move || viewer.stop()).await;
    }
    drop(subscription.control);
    Ok(())
}

#[cfg(test)]
mod tests {
    use brp_proto::constants::AUDIO_PRESET_ID;
    use brp_proto::{FrameHeader, FrameKind};

    use super::*;

    fn packet(len: usize) -> ReceivedFrame {
        ReceivedFrame::new(
            FrameHeader {
                live_id: 0,
                preset_id: AUDIO_PRESET_ID,
                kind: FrameKind::Audio,
                seq: 0,
                capture_ts_us: 0,
                keyframe: true,
                len: len as u32,
            },
            vec![0; len],
        )
    }

    #[tokio::test]
    async fn a_muted_person_is_counted_but_not_forwarded() {
        let (in_tx, in_rx) = mpsc::channel(8);
        let (out_tx, mut out_rx) = mpsc::channel(8);
        let hearing = Arc::new(AtomicBool::new(false));
        let bytes = Arc::new(AtomicU64::new(0));
        tokio::spawn(forward_heard(in_rx, out_tx, hearing.clone(), bytes.clone()));

        for len in [10, 20, 30] {
            in_tx.send(packet(len)).await.unwrap();
        }
        in_tx.send(packet(5)).await.unwrap();
        while bytes.load(Ordering::Relaxed) < 65 {
            tokio::task::yield_now().await;
        }
        hearing.store(true, Ordering::Relaxed);
        in_tx.send(packet(7)).await.unwrap();

        let heard = out_rx.recv().await.expect("the unmuted frame comes out");
        assert_eq!(heard.payload.len(), 7);
        assert_eq!(bytes.load(Ordering::Relaxed), 72);
        assert!(out_rx.try_recv().is_err(), "the muted frames never did");
    }
}
