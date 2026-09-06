use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brp_net::{
    AllowAll, AudioSubscription, LiveSource, MediaClient, MediaServer, NetError, PathKind,
    RelaySetting, SubscribeRejected, Subscription, bind_endpoint,
};
use brp_proto::constants::{AUDIO_PRESET_ID, MEDIA_ALPN};
use brp_proto::{AudioParams, Codec, CodecParams, EncodedFrame, FrameKind, ViewerMessage};
use iroh::SecretKey;
use iroh::protocol::Router;
use tokio::sync::mpsc;

struct ScriptedSource {
    params: CodecParams,
    frames: Mutex<Option<mpsc::Receiver<Arc<EncodedFrame>>>>,
    audio: Mutex<Option<mpsc::Receiver<Arc<EncodedFrame>>>>,
    keyframe_requests: AtomicUsize,
}

impl LiveSource for ScriptedSource {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        if live_id != 1 {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        if preset_id != 1 {
            return Err(SubscribeRejected::UnknownPreset(preset_id));
        }
        let frames = self
            .frames
            .lock()
            .unwrap()
            .take()
            .expect("single subscription");
        Ok(Subscription {
            params: self.params.clone(),
            frames,
        })
    }

    fn request_keyframe(&self, _live_id: u32, _preset_id: u32) {
        self.keyframe_requests.fetch_add(1, Ordering::SeqCst);
    }

    fn subscribe_audio(&self, live_id: u32) -> Result<AudioSubscription, SubscribeRejected> {
        if live_id != 1 {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        self.audio
            .lock()
            .unwrap()
            .take()
            .map(|packets| AudioSubscription {
                params: AudioParams::STANDARD,
                packets,
            })
            .ok_or(SubscribeRejected::NoAudio)
    }
}

fn params() -> CodecParams {
    CodecParams {
        codec: Codec::Hevc,
        width: 640,
        height: 360,
        fps: 30,
        extradata: vec![1, 2, 3],
    }
}

#[tokio::test]
async fn frames_travel_from_source_to_viewer_over_loopback() {
    let (tx, rx) = mpsc::channel(8);
    let source = Arc::new(ScriptedSource {
        params: params(),
        frames: Mutex::new(Some(rx)),
        audio: Mutex::new(None),
        keyframe_requests: AtomicUsize::new(0),
    });

    let server_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();
    let router = Router::builder(server_ep.clone())
        .accept(
            MEDIA_ALPN,
            MediaServer::new(source.clone(), Arc::new(AllowAll)),
        )
        .spawn();
    let client_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();

    let client = MediaClient::connect(&client_ep, server_ep.addr())
        .await
        .unwrap();
    let mut sub = client.subscribe(1, 1, false).await.unwrap();
    assert_eq!(sub.params, params());
    assert_eq!(client.path_kind(), PathKind::Direct);

    for seq in 0..5u64 {
        let frame = EncodedFrame {
            seq,
            capture_ts_us: seq * 1000,
            keyframe: seq == 0,
            data: vec![seq as u8; 100 + seq as usize],
        };
        tx.send(Arc::new(frame)).await.unwrap();
    }
    for seq in 0..5u64 {
        let received = tokio::time::timeout(Duration::from_secs(5), sub.frames.recv())
            .await
            .expect("frame in time")
            .expect("channel open");
        assert_eq!(received.header.seq, seq);
        assert_eq!(received.header.kind, FrameKind::Video);
        assert_eq!((received.header.live_id, received.header.preset_id), (1, 1));
        assert_eq!(received.header.keyframe, seq == 0);
        assert_eq!(received.header.len as usize, received.payload.len());
        assert_eq!(received.payload, vec![seq as u8; 100 + seq as usize]);
    }

    sub.control
        .send(ViewerMessage::RequestKeyframe)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while source.keyframe_requests.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("keyframe request reached the source");

    let rejected = client.subscribe(2, 1, false).await;
    assert!(
        matches!(rejected, Err(NetError::Rejected(ref reason)) if reason.contains("unknown live 2")),
        "{rejected:?}"
    );

    drop(tx);
    let ended = tokio::time::timeout(Duration::from_secs(5), sub.events.recv())
        .await
        .expect("event in time");
    assert!(matches!(
        ended,
        Some(brp_proto::PublisherMessage::LiveEnded)
    ));

    client.close();
    router.shutdown().await.unwrap();
    client_ep.close().await;
}

#[tokio::test]
async fn strangers_are_refused_before_any_subscription() {
    let (_tx, rx) = mpsc::channel(8);
    let source = Arc::new(ScriptedSource {
        params: params(),
        frames: Mutex::new(Some(rx)),
        audio: Mutex::new(None),
        keyframe_requests: AtomicUsize::new(0),
    });
    let member_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();
    let stranger_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();
    let member_id = member_ep.id();
    let policy = Arc::new(move |peer: iroh::EndpointId| peer == member_id);

    let server_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();
    let router = Router::builder(server_ep.clone())
        .accept(MEDIA_ALPN, MediaServer::new(source, policy))
        .spawn();

    let stranger = MediaClient::connect(&stranger_ep, server_ep.addr())
        .await
        .unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(5), stranger.subscribe(1, 1, false))
        .await
        .expect("refusal arrives promptly");
    assert!(
        matches!(
            refused,
            Err(NetError::Stream(_)) | Err(NetError::Connection(_))
        ),
        "{refused:?}"
    );

    let member = MediaClient::connect(&member_ep, server_ep.addr())
        .await
        .unwrap();
    assert!(member.subscribe(1, 1, false).await.is_ok());

    router.shutdown().await.unwrap();
    member_ep.close().await;
    stranger_ep.close().await;
}

#[tokio::test]
async fn audio_packets_travel_on_their_own_route_when_asked_for() {
    let (tx, rx) = mpsc::channel(8);
    let (audio_tx, audio_rx) = mpsc::channel(8);
    let source = Arc::new(ScriptedSource {
        params: params(),
        frames: Mutex::new(Some(rx)),
        audio: Mutex::new(Some(audio_rx)),
        keyframe_requests: AtomicUsize::new(0),
    });

    let server_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();
    let router = Router::builder(server_ep.clone())
        .accept(
            MEDIA_ALPN,
            MediaServer::new(source.clone(), Arc::new(AllowAll)),
        )
        .spawn();
    let client_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();

    let client = MediaClient::connect(&client_ep, server_ep.addr())
        .await
        .unwrap();

    let mut sub = client.subscribe(1, 1, true).await.unwrap();
    let audio = sub.audio.take().expect("audio granted");
    assert_eq!(audio.params, AudioParams::STANDARD);
    audio_tx
        .send(Arc::new(EncodedFrame {
            seq: 0,
            capture_ts_us: 5,
            keyframe: true,
            data: vec![9, 9, 9],
        }))
        .await
        .unwrap();
    tx.send(Arc::new(EncodedFrame {
        seq: 0,
        capture_ts_us: 5,
        keyframe: true,
        data: vec![1],
    }))
    .await
    .unwrap();
    let mut packets = audio.packets;
    let packet = tokio::time::timeout(Duration::from_secs(5), packets.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packet.header.kind, FrameKind::Audio);
    assert_eq!(packet.header.preset_id, AUDIO_PRESET_ID);
    assert_eq!(packet.payload, vec![9, 9, 9]);
    let video = tokio::time::timeout(Duration::from_secs(5), sub.frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(video.header.kind, FrameKind::Video);

    // A second subscription without audio gets none even though the source has it.
    let plain = client.subscribe(1, 1, false).await;
    assert!(
        plain.is_err(),
        "the scripted source allows one video subscription"
    );

    client.close();
    router.shutdown().await.unwrap();
    client_ep.close().await;
}

#[tokio::test]
async fn asking_for_audio_a_source_lacks_yields_video_only() {
    let (tx, rx) = mpsc::channel(8);
    let source = Arc::new(ScriptedSource {
        params: params(),
        frames: Mutex::new(Some(rx)),
        audio: Mutex::new(None),
        keyframe_requests: AtomicUsize::new(0),
    });

    let server_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();
    let router = Router::builder(server_ep.clone())
        .accept(
            MEDIA_ALPN,
            MediaServer::new(source.clone(), Arc::new(AllowAll)),
        )
        .spawn();
    let client_ep = bind_endpoint(SecretKey::generate(), RelaySetting::Disabled, vec![])
        .await
        .unwrap();

    let client = MediaClient::connect(&client_ep, server_ep.addr())
        .await
        .unwrap();

    let sub = client.subscribe(1, 1, true).await.unwrap();
    assert!(sub.audio.is_none());

    drop(tx);
    client.close();
    router.shutdown().await.unwrap();
    client_ep.close().await;
}
