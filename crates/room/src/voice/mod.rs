mod listen;
mod rules;
mod send;

use listen::Listener;
use rules::{SelfVoice, desired_listeners};
use send::{VoiceSend, silence_packet};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use brp_audio::{AudioOutput, AudioOutputSession, VoiceCapture};
use brp_net::AudioSubscription;
use brp_net::SubscribeRejected;
use brp_pipeline::Mixer;
use brp_proto::VoiceState;
use iroh::PublicKey;
use tokio::runtime::Handle;

use crate::codecs::{DecoderFactory, EncoderFactory};
use crate::error::RoomError;
use crate::gossip::lock;
use crate::membership::Membership;
use crate::registry::ChangeNotify;
use crate::snapshot::OwnVoiceView;
use crate::watcher::Watcher;

struct Session {
    send: VoiceSend,
    mixer: Mixer,
    // Dropping it stops playback.
    _output: Box<dyn AudioOutputSession>,
    capture: Arc<dyn VoiceCapture>,
    listeners: HashMap<PublicKey, Listener>,
}

#[derive(Default)]
struct State {
    me: SelfVoice,
    session: Option<Session>,
    error: Option<String>,
}

/// Joining, leaving, muting and deafening, serialised under one lock so the microphone is never
/// opened and closed concurrently.
pub(crate) struct Voice {
    membership: Arc<Mutex<Membership>>,
    watcher: Arc<Watcher>,
    encoders: Arc<dyn EncoderFactory>,
    decoders: Arc<dyn DecoderFactory>,
    runtime: Handle,
    notify: ChangeNotify,
    not_sending: Arc<Mutex<HashSet<PublicKey>>>,
    state: Mutex<State>,
}

impl Voice {
    pub fn new(
        membership: Arc<Mutex<Membership>>,
        watcher: Arc<Watcher>,
        encoders: Arc<dyn EncoderFactory>,
        decoders: Arc<dyn DecoderFactory>,
        runtime: Handle,
        notify: ChangeNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            membership,
            watcher,
            encoders,
            decoders,
            runtime,
            notify,
            not_sending: Arc::new(Mutex::new(HashSet::new())),
            state: Mutex::new(State::default()),
        })
    }

    pub fn join(
        &self,
        capture: Arc<dyn VoiceCapture>,
        output: Arc<dyn AudioOutput>,
    ) -> Result<(), RoomError> {
        {
            let mut state = lock(&self.state);
            if state.me.joined {
                return Ok(());
            }
            let mixer = Mixer::new();
            let output = output
                .start(mixer.render_fn())
                .map_err(RoomError::VoiceOutput)?;
            let encoder = self.encoders.open_voice()?;
            // A separate instance: encoding the probe frames would advance the live encoder's state.
            let silence = silence_packet(&mut *self.encoders.open_voice()?)?;
            let send = VoiceSend::start(
                encoder,
                silence,
                self.membership.clone(),
                self.not_sending.clone(),
                self.runtime.clone(),
            );
            state.me = state.me.join();
            match send.open_mic(&*capture) {
                Ok(()) => state.error = None,
                Err(error) => {
                    state.me = state.me.set_muted(true);
                    state.error = Some(error);
                }
            }
            state.session = Some(Session {
                send,
                mixer,
                _output: output,
                capture,
                listeners: HashMap::new(),
            });
            let stale = self.reconcile_locked(&mut state);
            drop(state);
            self.dispose(stale, None);
        }
        (self.notify)();
        Ok(())
    }

    pub fn leave(&self) {
        let (listeners, session) = {
            let mut state = lock(&self.state);
            let Some(mut session) = state.session.take() else {
                return;
            };
            session.send.close_mic();
            state.me = state.me.leave();
            state.error = None;
            (std::mem::take(&mut session.listeners), session)
        };
        self.dispose(listeners.into_values().collect(), Some(session.send));
        (self.notify)();
    }

    pub fn set_muted(&self, muted: bool) {
        self.change(|me| me.set_muted(muted));
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.change(|me| me.set_deafened(deafened));
    }

    /// Applies the periodic checks: who should be listened to, and whether the microphone died.
    pub fn reconcile(&self) {
        self.change(|me| me);
    }

    pub fn member_left(&self, id: PublicKey) {
        let listener = lock(&self.state)
            .session
            .as_mut()
            .and_then(|session| session.listeners.remove(&id));
        lock(&self.not_sending).remove(&id);
        self.dispose(listener.into_iter().collect(), None);
    }

    pub fn presence(&self) -> Option<VoiceState> {
        lock(&self.state).me.presence()
    }

    pub fn view(&self) -> OwnVoiceView {
        let state = lock(&self.state);
        OwnVoiceView {
            joined: state.me.joined,
            muted: state.me.muted,
            deafened: state.me.deafened,
            transmitting: state.me.transmits(),
            error: state.error.clone(),
        }
    }

    pub fn subscribe(&self, peer: PublicKey) -> Result<AudioSubscription, SubscribeRejected> {
        match &lock(&self.state).session {
            Some(session) => session.send.subscribe(peer),
            None => Err(SubscribeRejected::NoVoice),
        }
    }

    fn change(&self, apply: impl FnOnce(SelfVoice) -> SelfVoice) {
        let (changed, stale) = {
            let mut state = lock(&self.state);
            let before = (
                state.me,
                state.error.clone(),
                state.session.as_ref().map(|s| s.listeners.len()),
            );
            let was_transmitting = state.me.transmits();
            state.me = apply(state.me);
            self.sync_mic(&mut state, was_transmitting);
            let stale = self.reconcile_locked(&mut state);
            let after = (
                state.me,
                state.error.clone(),
                state.session.as_ref().map(|s| s.listeners.len()),
            );
            (before != after, stale)
        };
        self.dispose(stale, None);
        if changed {
            (self.notify)();
        }
    }

    fn sync_mic(&self, state: &mut State, was_transmitting: bool) {
        let Some(session) = &state.session else {
            return;
        };
        let transmits = state.me.transmits();
        if transmits && !was_transmitting {
            match session.send.open_mic(&*session.capture) {
                Ok(()) => state.error = None,
                Err(error) => {
                    state.me = state.me.set_muted(true);
                    state.error = Some(error);
                }
            }
        } else if transmits {
            if let Some(error) = session.send.mic_error() {
                session.send.close_mic();
                state.me = state.me.set_muted(true);
                state.error = Some(error);
            }
        } else if was_transmitting {
            session.send.close_mic();
        }
    }

    /// Returns the listeners to stop, which blocks and so happens outside the lock.
    fn reconcile_locked(&self, state: &mut State) -> Vec<Listener> {
        let me = state.me;
        let Some(session) = &mut state.session else {
            return Vec::new();
        };
        let desired = desired_listeners(
            me,
            lock(&self.membership)
                .members()
                .map(|m| (m.id, m.presence.voice.as_ref()))
                .collect::<Vec<_>>()
                .into_iter(),
        );
        let gone: Vec<PublicKey> = session
            .listeners
            .keys()
            .filter(|id| !desired.contains(*id))
            .copied()
            .collect();
        let stale = gone
            .iter()
            .filter_map(|id| session.listeners.remove(id))
            .collect();
        for peer in desired {
            session.listeners.entry(peer).or_insert_with(|| {
                Listener::start(
                    peer,
                    self.watcher.clone(),
                    self.decoders.clone(),
                    session.mixer.clone(),
                    true,
                    self.runtime.clone(),
                )
            });
        }
        stale
    }

    // Both stops join a thread, so they run off whichever executor thread called in.
    fn dispose(&self, listeners: Vec<Listener>, send: Option<VoiceSend>) {
        if listeners.is_empty() && send.is_none() {
            return;
        }
        self.runtime.spawn_blocking(move || {
            for listener in listeners {
                listener.stop();
            }
            if let Some(send) = send {
                send.stop();
            }
        });
    }
}
