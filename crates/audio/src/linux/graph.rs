//! Which PipeWire objects to link: application output nodes, minus brp's own, port by channel.
//! Pure bookkeeping so the decisions are testable without a server.

use std::collections::BTreeMap;

use crate::selection::{AppKey, AudioSelection, AudioSource};

const APP_OUTPUT_CLASS: &str = "Stream/Output/Audio";

/// The `node.name` our own capture stream registers under.
pub const OWN_STREAM_NAME: &str = "brp-audio-capture";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: u32,
    pub media_class: String,
    pub name: Option<String>,
    /// The `client.id` property; resolved to a pid through [`Graph::add_client`].
    pub client: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub id: u32,
    pub node: u32,
    pub direction_out: bool,
    pub channel: String,
}

/// What a Client global says about the process behind the nodes that name it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Client {
    /// The client's kernel-verified pid (`pipewire.sec.pid`).
    pub pid: Option<u32>,
    /// `application.process.binary`, or the basename of `/proc/<pid>/exe` when that property is
    /// absent. `None` when neither is available, which fails closed under a selection.
    pub key: Option<AppKey>,
    /// `application.name`, the friendly label the picker shows.
    pub label: Option<String>,
}

/// Which of our two capture inputs a port feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkPlan {
    pub port: u32,
    pub node: u32,
    pub input: Input,
}

/// What a newly seen node means for capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeVerdict {
    /// Our own capture stream: matched by name *and* by the pid resolved through its client.
    /// A name match alone is not enough — two brp instances on one machine both register a node
    /// under this name, and only the one whose pid is ours is the stream we own.
    Own,
    /// An application output node from another process; tracked, its ports link as they arrive.
    Linked,
    /// An application output node whose owning pid could not be resolved (its client is unknown,
    /// or the client itself has no resolvable pid). Left unlinked: missing a participant's audio
    /// is safer than looping our own back in on an unverified guess.
    Unresolved,
    /// Not an application output node, or one of our own excluded by resolved pid even without a
    /// name match (e.g. our own cpal playback stream).
    Ignored,
    /// An application output node whose identity is not in the selection, or has no identity while
    /// a selection is in force. Distinct from `Ignored` so logs and tests can tell "not audio"
    /// from "not chosen".
    NotSelected,
}

pub struct Graph {
    own_process_id: u32,
    /// brp's own executable identity, excluded in both modes.
    own_binary: AppKey,
    selection: AudioSelection,
    /// `client.id` to what that Client global reported.
    clients: BTreeMap<u32, Client>,
    /// Application output nodes that are not ours and are selected.
    nodes: BTreeMap<u32, Node>,
    /// Every output port seen, by id; ports can arrive before their node.
    ports: BTreeMap<u32, Port>,
}

impl Graph {
    pub fn new(own_process_id: u32, own_binary: AppKey, selection: AudioSelection) -> Self {
        Self {
            own_process_id,
            own_binary,
            selection,
            clients: BTreeMap::new(),
            nodes: BTreeMap::new(),
            ports: BTreeMap::new(),
        }
    }

    /// Records what a Client global reported. A client with no resolvable pid leaves the nodes it
    /// owns `Unresolved` rather than silently matching none.
    pub fn add_client(&mut self, id: u32, client: Client) {
        self.clients.insert(id, client);
    }

    /// Classifies a node and, if it is a foreign application output this session carries, tracks
    /// it for linking.
    pub fn add_node(&mut self, node: Node) -> NodeVerdict {
        let client = node.client.and_then(|id| self.clients.get(&id));
        let pid = client.and_then(|client| client.pid);
        let key = client.and_then(|client| client.key.clone());
        if node.name.as_deref() == Some(OWN_STREAM_NAME) && pid == Some(self.own_process_id) {
            return NodeVerdict::Own;
        }
        if node.media_class != APP_OUTPUT_CLASS {
            return NodeVerdict::Ignored;
        }
        // Both exclusions run ahead of the predicate, so neither our own playback nor a second brp
        // instance on this machine can be linked even if a hand-edited settings file names brp.
        if pid == Some(self.own_process_id) || key.as_ref() == Some(&self.own_binary) {
            return NodeVerdict::Ignored;
        }
        if pid.is_none() {
            return NodeVerdict::Unresolved;
        }
        if !self.selection.admits(key.as_ref()) {
            return NodeVerdict::NotSelected;
        }
        self.nodes.insert(node.id, node);
        NodeVerdict::Linked
    }

    /// The applications behind the tracked nodes, collapsed by identity: what `sources()` reports.
    /// A node whose owner has no identity cannot be selected, so it cannot be a row either. The
    /// first label seen for an identity wins; nodes are keyed by id, so that is deterministic.
    pub fn sources(&self) -> Vec<AudioSource> {
        let mut labels: BTreeMap<AppKey, String> = BTreeMap::new();
        for node in self.nodes.values() {
            let Some(client) = node.client.and_then(|id| self.clients.get(&id)) else {
                continue;
            };
            let Some(key) = client.key.clone() else {
                continue;
            };
            let label = match client.label.as_deref() {
                Some(label) if !label.is_empty() => label.to_string(),
                _ => key.as_str().to_string(),
            };
            labels.entry(key).or_insert(label);
        }
        labels
            .into_iter()
            .map(|(key, label)| AudioSource { key, label })
            .collect()
    }

    /// The link to make for this port now, if its node is already tracked.
    pub fn add_port(&mut self, port: Port) -> Option<LinkPlan> {
        if !port.direction_out {
            return None;
        }
        let plan = self.plan(&port);
        self.ports.insert(port.id, port);
        plan
    }

    /// Links for ports that arrived before their node did.
    pub fn pending_links(&self, node: u32) -> Vec<LinkPlan> {
        self.ports
            .values()
            .filter(|p| p.node == node)
            .filter_map(|p| self.plan(p))
            .collect()
    }

    pub fn remove(&mut self, id: u32) {
        self.nodes.remove(&id);
        self.ports.remove(&id);
        self.ports.retain(|_, p| p.node != id);
        self.clients.remove(&id);
    }

    fn plan(&self, port: &Port) -> Option<LinkPlan> {
        if !self.nodes.contains_key(&port.node) {
            return None;
        }
        let input = match port.channel.as_str() {
            "FL" => Input::Left,
            "FR" => Input::Right,
            "MONO" => Input::Both,
            _ => return None,
        };
        Some(LinkPlan {
            port: port.id,
            node: port.node,
            input,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN_PID: u32 = 4242;
    const OWN_BINARY: &str = "brp";

    fn graph(selection: AudioSelection) -> Graph {
        Graph::new(OWN_PID, AppKey::new(OWN_BINARY), selection)
    }

    fn only(binaries: [&str; 1]) -> AudioSelection {
        AudioSelection::Only(binaries.iter().map(|b| AppKey::new(b)).collect())
    }

    /// A client that reports a pid and nothing else, as phase 4's tests assumed.
    fn client(pid: u32) -> Client {
        Client {
            pid: Some(pid),
            ..Default::default()
        }
    }

    /// A client that also names its executable and itself.
    fn app(pid: u32, binary: &str, name: &str) -> Client {
        Client {
            pid: Some(pid),
            key: Some(AppKey::new(binary)),
            label: Some(name.into()),
        }
    }

    fn node(id: u32, client: Option<u32>) -> Node {
        Node {
            id,
            media_class: APP_OUTPUT_CLASS.into(),
            name: None,
            client,
        }
    }

    fn port(id: u32, node: u32, channel: &str) -> Port {
        Port {
            id,
            node,
            direction_out: true,
            channel: channel.into(),
        }
    }

    #[test]
    fn stereo_ports_of_foreign_apps_are_linked_by_channel() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, client(1000));
        assert_eq!(graph.add_node(node(10, Some(50))), NodeVerdict::Linked);
        assert_eq!(
            graph.add_port(port(11, 10, "FL")),
            Some(LinkPlan {
                port: 11,
                node: 10,
                input: Input::Left
            })
        );
        assert_eq!(
            graph.add_port(port(12, 10, "FR")),
            Some(LinkPlan {
                port: 12,
                node: 10,
                input: Input::Right
            })
        );
    }

    #[test]
    fn brps_own_nodes_and_non_application_nodes_are_ignored() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, client(OWN_PID));
        assert_eq!(graph.add_node(node(10, Some(50))), NodeVerdict::Ignored);
        assert_eq!(graph.add_port(port(11, 10, "FL")), None);
        let mut sink = node(20, None);
        sink.media_class = "Audio/Sink".into();
        assert_eq!(graph.add_node(sink), NodeVerdict::Ignored);
        assert_eq!(graph.add_port(port(21, 20, "FL")), None);
    }

    #[test]
    fn a_mono_port_feeds_both_inputs_and_input_ports_are_skipped() {
        let mut graph = Graph::new(1, AppKey::new(OWN_BINARY), AudioSelection::All);
        graph.add_client(50, client(999));
        graph.add_node(node(10, Some(50)));
        assert_eq!(
            graph.add_port(port(11, 10, "MONO")),
            Some(LinkPlan {
                port: 11,
                node: 10,
                input: Input::Both
            })
        );
        let mut input = port(12, 10, "FL");
        input.direction_out = false;
        assert_eq!(graph.add_port(input), None);
        assert_eq!(
            graph.add_port(port(13, 10, "RL")),
            None,
            "surround extras are not linked"
        );
    }

    #[test]
    fn a_port_seen_before_its_node_is_linked_when_the_node_arrives() {
        let mut graph = Graph::new(1, AppKey::new(OWN_BINARY), AudioSelection::All);
        graph.add_client(50, client(999));
        assert_eq!(graph.add_port(port(11, 10, "FL")), None);
        assert_eq!(graph.add_node(node(10, Some(50))), NodeVerdict::Linked);
        assert_eq!(
            graph.pending_links(10),
            vec![LinkPlan {
                port: 11,
                node: 10,
                input: Input::Left
            }]
        );
        graph.remove(10);
        assert!(graph.pending_links(10).is_empty());
    }

    #[test]
    fn a_node_whose_client_arrived_first_with_our_pid_is_own() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, client(OWN_PID));
        let mut own = node(10, Some(50));
        own.media_class = "Stream/Input/Audio".into();
        own.name = Some(OWN_STREAM_NAME.into());
        assert_eq!(graph.add_node(own), NodeVerdict::Own);
    }

    #[test]
    fn a_node_whose_client_is_unknown_is_unresolved() {
        let mut graph = graph(AudioSelection::All);
        assert_eq!(graph.add_node(node(10, Some(99))), NodeVerdict::Unresolved);
        assert_eq!(
            graph.add_port(port(11, 10, "FL")),
            None,
            "an unresolved node is not linked"
        );
    }

    #[test]
    fn a_node_named_like_ours_from_a_foreign_pid_is_linked_as_foreign() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, client(999));
        let mut foreign = node(10, Some(50));
        foreign.name = Some(OWN_STREAM_NAME.into());
        assert_eq!(graph.add_node(foreign), NodeVerdict::Linked);
    }

    #[test]
    fn only_the_selected_binary_is_linked() {
        let mut graph = graph(only(["firefox"]));
        graph.add_client(50, app(1000, "firefox", "Firefox"));
        graph.add_client(51, app(1001, "spotify", "Spotify"));
        assert_eq!(graph.add_node(node(10, Some(50))), NodeVerdict::Linked);
        assert_eq!(graph.add_node(node(20, Some(51))), NodeVerdict::NotSelected);
        assert_eq!(
            graph.add_port(port(21, 20, "FL")),
            None,
            "an unselected node is not linked"
        );
    }

    #[test]
    fn an_unresolvable_binary_links_under_all_and_is_not_selected_under_only() {
        let mut all = graph(AudioSelection::All);
        all.add_client(50, client(1000));
        assert_eq!(all.add_node(node(10, Some(50))), NodeVerdict::Linked);

        let mut restricted = graph(only(["firefox"]));
        restricted.add_client(50, client(1000));
        assert_eq!(
            restricted.add_node(node(10, Some(50))),
            NodeVerdict::NotSelected,
            "no identity means not selected: fail closed"
        );
    }

    #[test]
    fn our_own_pid_and_our_own_binary_are_excluded_in_both_modes() {
        for selection in [AudioSelection::All, only([OWN_BINARY])] {
            let mut graph = graph(selection);
            graph.add_client(50, app(OWN_PID, OWN_BINARY, "brp"));
            graph.add_client(51, app(9999, OWN_BINARY, "brp"));
            graph.add_client(
                52,
                Client {
                    pid: None,
                    key: Some(AppKey::new(OWN_BINARY)),
                    label: None,
                },
            );
            assert_eq!(
                graph.add_node(node(10, Some(50))),
                NodeVerdict::Ignored,
                "our own playback"
            );
            assert_eq!(
                graph.add_node(node(20, Some(51))),
                NodeVerdict::Ignored,
                "a second brp instance is an echo path, not an application"
            );
            assert_eq!(
                graph.add_node(node(30, Some(52))),
                NodeVerdict::Ignored,
                "excluded by name even without a resolvable pid"
            );
            assert!(graph.sources().is_empty(), "brp is never listed");
        }
    }

    #[test]
    fn nodes_of_one_binary_collapse_into_one_source() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, app(1000, "firefox", "Firefox"));
        graph.add_client(51, app(1001, "firefox", "Firefox"));
        graph.add_client(52, app(1002, "spotify", ""));
        graph.add_client(53, client(1003));
        for (id, client) in [(10, 50), (11, 50), (12, 51), (20, 52), (30, 53)] {
            graph.add_node(node(id, Some(client)));
        }
        assert_eq!(
            graph.sources(),
            vec![
                AudioSource {
                    key: AppKey::new("firefox"),
                    label: "Firefox".into(),
                },
                AudioSource {
                    key: AppKey::new("spotify"),
                    label: "spotify".into(),
                },
            ],
            "three Firefox streams are one row, an empty name falls back to the key, and a stream \
             with no identity cannot be listed"
        );
    }

    #[test]
    fn an_unselected_node_is_still_listed() {
        let mut graph = graph(AudioSelection::All);
        graph.add_client(50, app(1000, "spotify", "Spotify"));
        graph.add_node(node(10, Some(50)));
        assert_eq!(graph.sources().len(), 1, "the list reports what is audible");
    }
}
