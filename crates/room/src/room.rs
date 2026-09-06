//! Wires endpoint, gossip, media server, registry, and watcher together behind one handle.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_audio::{AudioCapture, AudioOutput, AudioOutputSession};
use brp_capture::{CaptureBackend, SourceId, SourceListing, SourceRequest};
use brp_net::{MediaServer, RelaySetting, bind_endpoint};
use brp_pipeline::{FrameNotify, Mixer};
use brp_proto::constants::{
    ENCODER_IDLE_STOP_GRACE, JOIN_TIMEOUT, MAX_LIVES_PER_PARTICIPANT, MEDIA_ALPN, MEMBER_EXPIRY,
    NICKNAME_MAX_LEN, PRESENCE_HEARTBEAT, REGISTRY_HOUSEKEEPING,
};
use brp_proto::{Preset, RoomTicket, SourceKind, template_presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, EndpointId, PublicKey, SecretKey};
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::codecs::{DecoderFactory, EncoderFactory};
use crate::error::RoomError;
use crate::gossip::{self, PresenceLoop, lock};
use crate::membership::Membership;
use crate::registry::{CaptureFan, ChangeNotify, LiveRegistry};
use crate::snapshot::{MemberView, RoomSnapshot};
use crate::watcher::{WatchHandle, Watcher};

#[derive(Debug, Clone, Copy)]
pub struct RoomTimings {
    pub heartbeat: Duration,
    pub expiry: Duration,
    pub housekeeping: Duration,
    pub encoder_grace: Duration,
    pub join_timeout: Duration,
}

impl Default for RoomTimings {
    fn default() -> Self {
        Self {
            heartbeat: PRESENCE_HEARTBEAT,
            expiry: MEMBER_EXPIRY,
            housekeeping: REGISTRY_HOUSEKEEPING,
            encoder_grace: ENCODER_IDLE_STOP_GRACE,
            join_timeout: JOIN_TIMEOUT,
        }
    }
}

pub struct RoomConfig {
    pub secret: SecretKey,
    pub relay: RelaySetting,
    pub nickname: String,
    pub target_fps: u32,
    pub capture: Arc<dyn CaptureBackend>,
    pub audio_capture: Arc<dyn AudioCapture>,
    pub audio_output: Arc<dyn AudioOutput>,
    pub encoders: Arc<dyn EncoderFactory>,
    pub decoders: Arc<dyn DecoderFactory>,
    pub on_change: ChangeNotify,
    pub on_frame: FrameNotify,
    pub timings: RoomTimings,
}

pub struct Room {
    me: PublicKey,
    nickname: String,
    topic: [u8; 32],
    endpoint: Endpoint,
    router: Router,
    membership: Arc<Mutex<Membership>>,
    registry: Arc<LiveRegistry>,
    watcher: Arc<Watcher>,
    capture: Arc<dyn CaptureBackend>,
    encoders: Arc<dyn EncoderFactory>,
    target_fps: u32,
    version: Arc<AtomicU64>,
    notify: ChangeNotify,
    mixer: Mixer,
    /// Kept alive for the room's lifetime; dropping it stops playback.
    _audio_output: Option<Box<dyn AudioOutputSession>>,
    audio_output_error: Option<String>,
    tasks: Vec<JoinHandle<()>>,
}

impl Room {
    pub async fn create(config: RoomConfig) -> Result<Self, RoomError> {
        Self::start(config, RoomTicket::random_topic(), Vec::new()).await
    }

    pub async fn join(config: RoomConfig, ticket: RoomTicket) -> Result<Self, RoomError> {
        Self::start(config, ticket.topic, ticket.bootstrap).await
    }

    async fn start(
        config: RoomConfig,
        topic: [u8; 32],
        bootstrap: Vec<EndpointAddr>,
    ) -> Result<Self, RoomError> {
        let nickname: String = config.nickname.chars().take(NICKNAME_MAX_LEN).collect();
        let me = config.secret.public();
        let endpoint =
            bind_endpoint(config.secret.clone(), config.relay, bootstrap.clone()).await?;

        let version = Arc::new(AtomicU64::new(0));
        let dirty = Arc::new(Notify::new());
        let notify: ChangeNotify = {
            let version = version.clone();
            let callback = config.on_change.clone();
            Arc::new(move || {
                version.fetch_add(1, Ordering::Relaxed);
                callback();
            })
        };
        // Registry changes alter our presence, so they also wake the broadcaster.
        let registry_notify: ChangeNotify = {
            let notify = notify.clone();
            let dirty = dirty.clone();
            Arc::new(move || {
                notify();
                dirty.notify_one();
            })
        };
        let membership = Arc::new(Mutex::new(Membership::new(config.timings.expiry)));
        let registry = LiveRegistry::new(
            config.encoders.clone(),
            config.audio_capture.clone(),
            config.timings.encoder_grace,
            registry_notify,
        );
        let policy = {
            let membership = membership.clone();
            Arc::new(move |peer: EndpointId| lock(&membership).is_member(&peer))
        };

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(MEDIA_ALPN, MediaServer::new(registry.clone(), policy))
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();

        let bootstrap_ids: Vec<EndpointId> = bootstrap.iter().map(|addr| addr.id).collect();
        let (sender, receiver) = gossip::join(
            &gossip,
            TopicId::from_bytes(topic),
            bootstrap_ids,
            config.timings.join_timeout,
        )
        .await?;

        let (expired_tx, mut expired_rx) = mpsc::channel::<PublicKey>(16);
        let (audio_changed_tx, mut audio_changed_rx) = mpsc::channel::<PublicKey>(16);
        let presence = PresenceLoop {
            secret: config.secret,
            nickname: nickname.clone(),
            sender,
            receiver,
            membership: membership.clone(),
            registry: registry.clone(),
            dirty,
            heartbeat: config.timings.heartbeat,
            on_change: notify.clone(),
            expired: expired_tx,
            audio_changed: audio_changed_tx,
        };
        let housekeeping = {
            let registry = registry.clone();
            let every = config.timings.housekeeping;
            async move {
                let mut tick = tokio::time::interval(every);
                loop {
                    tick.tick().await;
                    registry.housekeeping(Instant::now());
                }
            }
        };
        let mixer = Mixer::new();
        let (audio_output, audio_output_error) = match config.audio_output.start(mixer.render_fn())
        {
            Ok(session) => (Some(session), None),
            Err(error) => {
                tracing::warn!(%error, "audio output unavailable; watches will not ask for audio");
                (None, Some(error.to_string()))
            }
        };
        let watcher = Watcher::new(
            endpoint.clone(),
            tokio::runtime::Handle::current(),
            config.decoders.clone(),
            membership.clone(),
            mixer.clone(),
            audio_output_error.is_none(),
            notify.clone(),
            config.on_frame.clone(),
        );
        let expiry_consumer = {
            let watcher = watcher.clone();
            async move {
                while let Some(id) = expired_rx.recv().await {
                    watcher.member_left(id);
                }
            }
        };
        let audio_consumer = {
            let watcher = watcher.clone();
            async move {
                while let Some(id) = audio_changed_rx.recv().await {
                    watcher.reacquire_audio(id);
                }
            }
        };
        let tasks = vec![
            tokio::spawn(presence.run()),
            tokio::spawn(housekeeping),
            tokio::spawn(expiry_consumer),
            tokio::spawn(audio_consumer),
        ];

        Ok(Self {
            me,
            nickname,
            topic,
            endpoint,
            router,
            membership,
            registry,
            watcher,
            capture: config.capture,
            encoders: config.encoders,
            target_fps: config.target_fps,
            version,
            notify,
            mixer,
            _audio_output: audio_output,
            audio_output_error,
            tasks,
        })
    }

    pub fn id(&self) -> PublicKey {
        self.me
    }

    pub fn nickname(&self) -> &str {
        &self.nickname
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// A ticket listing this participant as bootstrap, so anyone online can invite.
    pub fn ticket(&self) -> RoomTicket {
        RoomTicket::new(self.topic, vec![self.endpoint.addr()])
    }

    /// Waits for relay registration so the ticket carries a relay address. Always bounded, because
    /// with relays disabled the transport never reports online.
    pub async fn online(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.endpoint.online())
            .await
            .is_ok()
    }

    pub fn snapshot(&self) -> RoomSnapshot {
        // Read before materialising members: a version must never be newer than the contents it
        // describes, so a change landing mid-snapshot forces a stale-version re-snapshot rather
        // than hiding new contents behind an already-current version.
        let version = self.version();
        let now = Instant::now();
        let members = lock(&self.membership)
            .members()
            .map(|m| MemberView {
                id: m.id,
                nickname: m.presence.nickname.clone(),
                has_audio: m.has_audio(),
                lives: m.presence.lives.clone(),
                seen_ago_ms: now.duration_since(m.last_seen).as_millis() as u64,
                path: self.watcher.path_kind(&m.id),
                gain: self.mixer.gain(m.id.as_bytes()),
            })
            .collect();
        RoomSnapshot {
            me: self.me,
            nickname: self.nickname.clone(),
            version,
            members,
            own_lives: self.registry.views(),
            watches: self.watcher.views(),
            own_audio: self.registry.audio_view(),
            audio_output_error: self.audio_output_error.clone(),
            master_mute: self.mixer.muted(),
        }
    }

    /// Lists what the platform can share, or says that it picks for itself.
    pub fn sources(&self, kind: SourceKind) -> Result<SourceListing, RoomError> {
        Ok(self.capture.sources(kind)?)
    }

    pub async fn start_live(
        &self,
        kind: SourceKind,
        source: Option<SourceId>,
        title: String,
    ) -> Result<u32, RoomError> {
        // Cheap check before capture opens a session (a portal permission dialog for real users),
        // so a session isn't opened only to be rejected once `add_live` re-checks the same cap.
        if self.registry.live_count() >= MAX_LIVES_PER_PARTICIPANT {
            return Err(RoomError::TooManyLives);
        }
        let fan = Arc::new(CaptureFan::default());
        let sink = fan.clone();
        let session = self
            .capture
            .start(
                SourceRequest {
                    kind,
                    source,
                    target_fps: self.target_fps,
                },
                Box::new(move |frame| sink.push(frame)),
            )
            .await?;
        let info = session.info();
        let fps = info.fps.min(self.target_fps).max(1);
        let presets = template_presets(
            info.width,
            info.height,
            fps,
            self.encoders.preferred_codec(),
        );
        self.registry.add_live(title, kind, session, fan, presets)
    }

    pub fn stop_live(&self, live_id: u32) -> Result<(), RoomError> {
        self.registry.remove_live(live_id)
    }

    pub fn set_presets(&self, live_id: u32, presets: Vec<Preset>) -> Result<(), RoomError> {
        self.registry.set_presets(live_id, presets)
    }

    pub fn watch(
        &self,
        publisher: PublicKey,
        live_id: u32,
        preset_id: u32,
    ) -> Result<WatchHandle, RoomError> {
        self.watcher.watch(publisher, live_id, preset_id)
    }

    pub fn unwatch(&self, publisher: PublicKey, live_id: u32) -> Result<(), RoomError> {
        self.watcher.unwatch(publisher, live_id)
    }

    pub fn set_audio(&self, enabled: bool) {
        self.registry.set_audio(enabled);
    }

    pub fn set_volume(&self, publisher: PublicKey, gain: f32) {
        self.mixer.set_gain(*publisher.as_bytes(), gain);
        (self.notify)();
    }

    pub fn set_master_mute(&self, muted: bool) {
        self.mixer.set_muted(muted);
        (self.notify)();
    }

    pub async fn leave(mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.watcher.stop_all();
        self.registry.stop_all();
        if let Err(error) = self.router.shutdown().await {
            tracing::warn!(%error, "router shutdown");
        }
        self.endpoint.close().await;
    }
}
