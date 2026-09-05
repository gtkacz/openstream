//! Wires endpoint, gossip, media server, registry, and watcher together behind one handle.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, SourceRequest};
use brp_net::{MediaServer, PathKind, RelaySetting, bind_endpoint};
use brp_pipeline::FrameNotify;
use brp_proto::constants::{
    ENCODER_IDLE_STOP_GRACE, JOIN_TIMEOUT, MEDIA_ALPN, MEMBER_EXPIRY, NICKNAME_MAX_LEN,
    PRESENCE_HEARTBEAT, REGISTRY_HOUSEKEEPING,
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
    capture: Arc<dyn CaptureBackend>,
    encoders: Arc<dyn EncoderFactory>,
    target_fps: u32,
    version: Arc<AtomicU64>,
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

        let (expired_tx, _expired_rx) = mpsc::channel::<PublicKey>(16);
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
        let tasks = vec![tokio::spawn(presence.run()), tokio::spawn(housekeeping)];

        Ok(Self {
            me,
            nickname,
            topic,
            endpoint,
            router,
            membership,
            registry,
            capture: config.capture,
            encoders: config.encoders,
            target_fps: config.target_fps,
            version,
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

    pub fn snapshot(&self) -> RoomSnapshot {
        let now = Instant::now();
        let members = lock(&self.membership)
            .members()
            .map(|m| MemberView {
                id: m.id,
                nickname: m.presence.nickname.clone(),
                lives: m.presence.lives.clone(),
                seen_ago_ms: now.duration_since(m.last_seen).as_millis() as u64,
                path: PathKind::Unknown,
            })
            .collect();
        RoomSnapshot {
            me: self.me,
            nickname: self.nickname.clone(),
            version: self.version(),
            members,
            own_lives: self.registry.views(),
            watches: Vec::new(),
        }
    }

    pub async fn start_live(&self, kind: SourceKind, title: String) -> Result<u32, RoomError> {
        let fan = Arc::new(CaptureFan::default());
        let sink = fan.clone();
        let session = self
            .capture
            .start(
                SourceRequest {
                    kind,
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

    pub async fn leave(mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.registry.stop_all();
        if let Err(error) = self.router.shutdown().await {
            tracing::warn!(%error, "router shutdown");
        }
        self.endpoint.close().await;
    }
}
