//! Which PipeWire objects to link: application output nodes, minus brp's own, port by channel.
//! Pure bookkeeping so the decisions are testable without a server.

use std::collections::BTreeMap;

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
}

pub struct Graph {
    own_process_id: u32,
    /// `client.id` to the client's kernel-verified pid (`pipewire.sec.pid`).
    client_pids: BTreeMap<u32, u32>,
    /// Application output nodes that are not ours.
    nodes: BTreeMap<u32, Node>,
    /// Every output port seen, by id; ports can arrive before their node.
    ports: BTreeMap<u32, Port>,
}

impl Graph {
    pub fn new(own_process_id: u32) -> Self {
        Self {
            own_process_id,
            client_pids: BTreeMap::new(),
            nodes: BTreeMap::new(),
            ports: BTreeMap::new(),
        }
    }

    /// Records a client's kernel-verified pid. A client with no resolvable pid is not recorded,
    /// so nodes it owns resolve to `Unresolved` rather than silently matching none.
    pub fn add_client(&mut self, id: u32, pid: Option<u32>) {
        if let Some(pid) = pid {
            self.client_pids.insert(id, pid);
        }
    }

    /// Classifies a node and, if it is a foreign application output, tracks it for linking.
    pub fn add_node(&mut self, node: Node) -> NodeVerdict {
        let pid = node
            .client
            .and_then(|client| self.client_pids.get(&client).copied());
        if node.name.as_deref() == Some(OWN_STREAM_NAME) && pid == Some(self.own_process_id) {
            return NodeVerdict::Own;
        }
        if node.media_class != APP_OUTPUT_CLASS {
            return NodeVerdict::Ignored;
        }
        match pid {
            None => NodeVerdict::Unresolved,
            Some(p) if p == self.own_process_id => NodeVerdict::Ignored,
            Some(_) => {
                self.nodes.insert(node.id, node);
                NodeVerdict::Linked
            }
        }
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
        self.client_pids.remove(&id);
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
        let mut graph = Graph::new(4242);
        graph.add_client(50, Some(1000));
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
        let mut graph = Graph::new(4242);
        graph.add_client(50, Some(4242));
        assert_eq!(graph.add_node(node(10, Some(50))), NodeVerdict::Ignored);
        assert_eq!(graph.add_port(port(11, 10, "FL")), None);
        let mut sink = node(20, None);
        sink.media_class = "Audio/Sink".into();
        assert_eq!(graph.add_node(sink), NodeVerdict::Ignored);
        assert_eq!(graph.add_port(port(21, 20, "FL")), None);
    }

    #[test]
    fn a_mono_port_feeds_both_inputs_and_input_ports_are_skipped() {
        let mut graph = Graph::new(1);
        graph.add_client(50, Some(999));
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
        let mut graph = Graph::new(1);
        graph.add_client(50, Some(999));
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
        let mut graph = Graph::new(4242);
        graph.add_client(50, Some(4242));
        let mut own = node(10, Some(50));
        own.media_class = "Stream/Input/Audio".into();
        own.name = Some(OWN_STREAM_NAME.into());
        assert_eq!(graph.add_node(own), NodeVerdict::Own);
    }

    #[test]
    fn a_node_whose_client_is_unknown_is_unresolved() {
        let mut graph = Graph::new(4242);
        assert_eq!(graph.add_node(node(10, Some(99))), NodeVerdict::Unresolved);
        assert_eq!(
            graph.add_port(port(11, 10, "FL")),
            None,
            "an unresolved node is not linked"
        );
    }

    #[test]
    fn a_node_named_like_ours_from_a_foreign_pid_is_linked_as_foreign() {
        let mut graph = Graph::new(4242);
        graph.add_client(50, Some(999));
        let mut foreign = node(10, Some(50));
        foreign.name = Some(OWN_STREAM_NAME.into());
        assert_eq!(graph.add_node(foreign), NodeVerdict::Linked);
    }
}
