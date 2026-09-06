//! Linux capture: a PipeWire stream linked to every application's playback node except brp's own,
//! so what we play from other participants never comes back to them.

pub mod graph;

use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::rc::Rc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use brp_proto::constants::{AUDIO_CAPTURE_START_TIMEOUT, AUDIO_CHANNELS, AUDIO_SAMPLE_RATE};
use brp_proto::monotonic_us;
use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::sys::{SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR, SPA_AUDIO_MAX_CHANNELS};
use pw::spa::utils::{Direction, SpaTypes};
use pw::types::ObjectType;

use self::graph::{Graph, Input, LinkPlan, Node, NodeVerdict, OWN_STREAM_NAME, Port};
use crate::chunk::{AudioCapture, AudioCaptureSession, AudioChunk, AudioSink};
use crate::error::AudioError;

pub struct PipeWireCapture {
    process_id: u32,
}

impl PipeWireCapture {
    pub fn new(process_id: u32) -> Self {
        Self { process_id }
    }
}

struct Session {
    quit: pw::channel::Sender<()>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture for PipeWireCapture {
    fn start(&self, sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let (quit_tx, quit_rx) = pw::channel::channel();
        let error = Arc::new(Mutex::new(None));
        let error_slot = error.clone();
        let process_id = self.process_id;
        let thread = thread::Builder::new()
            .name("brp-audio-pw".into())
            .spawn(move || {
                if let Err(e) = run(process_id, sink, quit_rx, ready_tx.clone()) {
                    let message = e.to_string();
                    let _ = ready_tx.send(Err(e));
                    *error_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                }
            })
            .map_err(|e| {
                AudioError::PipeWire(format!("failed to spawn the PipeWire thread: {e}"))
            })?;
        // The registry starts capture with its lock released, but a wedged daemon must not hold
        // the subscriber that asked for it either.
        match ready_rx.recv_timeout(AUDIO_CAPTURE_START_TIMEOUT) {
            Ok(result) => result?,
            Err(RecvTimeoutError::Timeout) => {
                let _ = quit_tx.send(());
                let _ = thread.join();
                return Err(AudioError::PipeWire(format!(
                    "the capture stream did not become ready within {AUDIO_CAPTURE_START_TIMEOUT:?}"
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AudioError::PipeWire(
                    "PipeWire thread exited before connecting".into(),
                ));
            }
        }
        Ok(Box::new(Session {
            quit: quit_tx,
            error,
            thread: Some(thread),
        }))
    }
}

impl AudioCaptureSession for Session {
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    fn stop(mut self: Box<Self>) {
        let _ = self.quit.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.quit.send(());
    }
}

/// Everything the registry callbacks need. The PipeWire main loop is single-threaded, so `Rc` and
/// `RefCell` are the right tools here.
struct State {
    core: pw::core::CoreRc,
    graph: RefCell<Graph>,
    /// Our capture stream's node id. `Stream::node_id` is unassigned until the stream is paused,
    /// so it is learned from the registry instead, via `Graph`'s `NodeVerdict::Own`.
    stream_node: RefCell<Option<u32>>,
    inputs: RefCell<Inputs>,
    /// Links we created, kept alive by holding the proxies.
    links: RefCell<Vec<pw::link::Link>>,
}

#[derive(Default)]
struct Inputs {
    left: Option<u32>,
    right: Option<u32>,
    /// Plans that arrived before our own ports existed.
    deferred: Vec<LinkPlan>,
}

fn run(
    process_id: u32,
    sink: AudioSink,
    quit: pw::channel::Receiver<()>,
    ready: mpsc::Sender<Result<(), AudioError>>,
) -> Result<(), AudioError> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(pw_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(pw_error)?;
    let core = context.connect_rc(None).map_err(pw_error)?;
    let registry = core.get_registry().map_err(pw_error)?;

    let stream = pw::stream::StreamBox::new(
        &core,
        OWN_STREAM_NAME,
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Production",
            *pw::keys::NODE_AUTOCONNECT => "false",
            // The `name` argument above only fills in `media.name`; `node.name` must be set
            // explicitly for the registry lookup below to recognise our own stream.
            *pw::keys::NODE_NAME => OWN_STREAM_NAME,
        },
    )
    .map_err(pw_error)?;
    let _listener = stream
        .add_local_listener_with_user_data(sink)
        .process(|stream, sink| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let offset = data.chunk().offset() as usize;
            let Some(bytes) = data.data() else { return };
            let Some(bytes) = bytes.get(offset..offset.saturating_add(size)) else {
                return;
            };
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            if !samples.is_empty() {
                sink(AudioChunk {
                    samples,
                    capture_ts_us: monotonic_us(),
                });
            }
        })
        .register()
        .map_err(pw_error)?;

    let format = format_pod();
    let Some(pod) = Pod::from_bytes(&format) else {
        return Err(AudioError::PipeWire("format pod did not serialize".into()));
    };
    let mut params = [pod];
    // No autoconnect and no target: the stream negotiates against its own adapter and waits for
    // the links we make.
    stream
        .connect(
            Direction::Input,
            None,
            pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(pw_error)?;

    let state = Rc::new(State {
        core: core.clone(),
        graph: RefCell::new(Graph::new(process_id)),
        stream_node: RefCell::new(None),
        inputs: RefCell::new(Inputs::default()),
        links: RefCell::new(Vec::new()),
    });
    let on_global = state.clone();
    let on_remove = state.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| on_global.global(global))
        .global_remove(move |id| on_remove.remove(id))
        .register();

    // Distinguishes a deliberate `Session::stop()`/`Drop` (expected exit, `Ok`) from the mainloop
    // returning for any other reason (daemon disconnect, core error) — the latter must surface
    // through the registry's `error()` poll, so it is reported as a failure.
    let quit_requested = Rc::new(Cell::new(false));
    let core_error = Rc::new(RefCell::new(None::<String>));
    let _core_listener = {
        let mainloop = mainloop.clone();
        let core_error = core_error.clone();
        core.add_listener_local()
            .error(move |id, seq, res, message| {
                tracing::warn!(id, seq, res, message, "PipeWire object reported an error");
                // Per-object errors are routine (e.g. a link target vanished between the
                // registry announcing it and our `create_object` racing it); only the core's
                // own error means the daemon connection itself is gone.
                if id == pw::core::PW_ID_CORE {
                    *core_error.borrow_mut() = Some(message.to_string());
                    mainloop.quit();
                }
            })
            .register()
    };

    // Our own node and both of its input ports come from the registry, and every link plan waits
    // for them. If the graph never gives us that verdict nothing is ever linked, no chunk ever
    // flows, and the session would otherwise report itself healthy for ever.
    let not_linkable = Rc::new(Cell::new(false));
    let _linkable_timer = {
        let quit_loop = mainloop.clone();
        let state = state.clone();
        let not_linkable = not_linkable.clone();
        let timer = mainloop.loop_().add_timer(move |_| {
            if !state.linkable() {
                not_linkable.set(true);
                quit_loop.quit();
            }
        });
        timer
            .update_timer(Some(AUDIO_CAPTURE_START_TIMEOUT), None)
            .into_result()
            .map_err(|e| AudioError::PipeWire(format!("could not arm the link deadline: {e}")))?;
        timer
    };

    let _ = ready.send(Ok(()));
    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        let quit_requested = quit_requested.clone();
        move |_| {
            quit_requested.set(true);
            mainloop.quit();
        }
    });
    mainloop.run();

    if quit_requested.get() {
        return Ok(());
    }
    if not_linkable.get() {
        let message = "capture stream never became linkable";
        tracing::warn!(
            "no PipeWire link could be made within {AUDIO_CAPTURE_START_TIMEOUT:?}; stopping audio capture"
        );
        return Err(AudioError::PipeWire(message.into()));
    }
    Err(match core_error.borrow_mut().take() {
        Some(message) => AudioError::PipeWire(message),
        None => AudioError::PipeWire("the PipeWire loop exited unexpectedly".into()),
    })
}

impl State {
    fn global(&self, global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>) {
        let Some(props) = global.props else { return };
        match global.type_ {
            ObjectType::Client => {
                let pid = props
                    .get(*pw::keys::SEC_PID)
                    .and_then(|pid| pid.parse().ok());
                self.graph.borrow_mut().add_client(global.id, pid);
            }
            ObjectType::Node => {
                let id = global.id;
                let name = props.get(*pw::keys::NODE_NAME).map(str::to_string);
                let node = Node {
                    id,
                    media_class: props.get(*pw::keys::MEDIA_CLASS).unwrap_or("").to_string(),
                    name: name.clone(),
                    client: props.get(*pw::keys::CLIENT_ID).and_then(|c| c.parse().ok()),
                };
                let verdict = self.graph.borrow_mut().add_node(node);
                match verdict {
                    NodeVerdict::Own => {
                        *self.stream_node.borrow_mut() = Some(id);
                    }
                    NodeVerdict::Linked => {
                        let plans = self.graph.borrow().pending_links(id);
                        for plan in plans {
                            self.link(plan);
                        }
                    }
                    NodeVerdict::Unresolved => {
                        tracing::warn!(
                            node = id,
                            name = name.as_deref().unwrap_or(""),
                            "could not resolve the pid owning this audio output node; leaving it unlinked"
                        );
                    }
                    NodeVerdict::Ignored => {}
                }
            }
            ObjectType::Port => {
                let Some(node) = props.get(*pw::keys::NODE_ID).and_then(|n| n.parse().ok()) else {
                    return;
                };
                let direction_out = props.get(*pw::keys::PORT_DIRECTION) == Some("out");
                let channel = props
                    .get(*pw::keys::AUDIO_CHANNEL)
                    .unwrap_or("")
                    .to_string();
                if Some(node) == *self.stream_node.borrow() {
                    self.own_port(global.id, direction_out, &channel);
                    return;
                }
                let port = Port {
                    id: global.id,
                    node,
                    direction_out,
                    channel,
                };
                if let Some(plan) = self.graph.borrow_mut().add_port(port) {
                    self.link(plan);
                }
            }
            _ => {}
        }
    }

    /// True once our own node and both of its input ports are known, which is what every link
    /// plan waits for.
    fn linkable(&self) -> bool {
        let inputs = self.inputs.borrow();
        self.stream_node.borrow().is_some() && inputs.left.is_some() && inputs.right.is_some()
    }

    fn own_port(&self, id: u32, direction_out: bool, channel: &str) {
        if direction_out {
            return;
        }
        let mut inputs = self.inputs.borrow_mut();
        match channel {
            "FL" => inputs.left = Some(id),
            "FR" => inputs.right = Some(id),
            _ => return,
        }
        if inputs.left.is_some() && inputs.right.is_some() {
            let deferred = std::mem::take(&mut inputs.deferred);
            drop(inputs);
            for plan in deferred {
                self.link(plan);
            }
        }
    }

    fn remove(&self, id: u32) {
        self.graph.borrow_mut().remove(id);
        // Links to a removed node die with it on the server; dropping our proxies just tidies up.
    }

    fn link(&self, plan: LinkPlan) {
        let (left, right) = {
            let inputs = self.inputs.borrow();
            (inputs.left, inputs.right)
        };
        let stream_node = *self.stream_node.borrow();
        let (Some(left), Some(right), Some(stream_node)) = (left, right, stream_node) else {
            self.inputs.borrow_mut().deferred.push(plan);
            return;
        };
        let targets: &[u32] = match plan.input {
            Input::Left => &[left],
            Input::Right => &[right],
            Input::Both => &[left, right],
        };
        for input in targets {
            let result = self.core.create_object::<pw::link::Link>(
                "link-factory",
                &pw::properties::properties! {
                    *pw::keys::LINK_OUTPUT_NODE => plan.node.to_string(),
                    *pw::keys::LINK_OUTPUT_PORT => plan.port.to_string(),
                    *pw::keys::LINK_INPUT_NODE => stream_node.to_string(),
                    *pw::keys::LINK_INPUT_PORT => input.to_string(),
                    // Passive links do not keep an idle application node running.
                    "link.passive" => "true",
                },
            );
            match result {
                Ok(link) => self.links.borrow_mut().push(link),
                Err(error) => {
                    tracing::warn!(node = plan.node, port = plan.port, %error, "could not link audio node")
                }
            }
        }
    }
}

fn format_pod() -> Vec<u8> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::F32LE);
    info.set_rate(AUDIO_SAMPLE_RATE);
    info.set_channels(u32::from(AUDIO_CHANNELS));
    let mut position = [0u32; SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = SPA_AUDIO_CHANNEL_FL;
    position[1] = SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("the audio format is a valid SPA object")
        .0
        .into_inner()
}

fn pw_error(error: pw::Error) -> AudioError {
    AudioError::PipeWire(error.to_string())
}
