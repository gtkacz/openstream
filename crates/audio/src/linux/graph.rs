//! Which PipeWire objects to link: application output nodes, minus brp's own, port by channel.
//! Pure bookkeeping so the decisions are testable without a server.

use std::collections::BTreeMap;

const APP_OUTPUT_CLASS: &str = "Stream/Output/Audio";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: u32,
    pub media_class: String,
    pub process_id: Option<u32>,
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

pub struct Graph {
    own_process_id: u32,
    /// Application output nodes that are not ours.
    nodes: BTreeMap<u32, Node>,
    /// Every output port seen, by id; ports can arrive before their node.
    ports: BTreeMap<u32, Port>,
}

impl Graph {
    pub fn new(own_process_id: u32) -> Self {
        Self {
            own_process_id,
            nodes: BTreeMap::new(),
            ports: BTreeMap::new(),
        }
    }

    /// True when the node is an application's playback stream from another process.
    pub fn add_node(&mut self, node: Node) -> bool {
        let wanted =
            node.media_class == APP_OUTPUT_CLASS && node.process_id != Some(self.own_process_id);
        if wanted {
            self.nodes.insert(node.id, node);
        }
        wanted
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

    fn node(id: u32, pid: Option<u32>) -> Node {
        Node {
            id,
            media_class: "Stream/Output/Audio".into(),
            process_id: pid,
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
        assert!(graph.add_node(node(10, Some(1000))));
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
        assert!(!graph.add_node(node(10, Some(4242))));
        assert_eq!(graph.add_port(port(11, 10, "FL")), None);
        let mut sink = node(20, None);
        sink.media_class = "Audio/Sink".into();
        assert!(!graph.add_node(sink));
        assert_eq!(graph.add_port(port(21, 20, "FL")), None);
    }

    #[test]
    fn a_mono_port_feeds_both_inputs_and_input_ports_are_skipped() {
        let mut graph = Graph::new(1);
        graph.add_node(node(10, None));
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
        assert_eq!(graph.add_port(port(11, 10, "FL")), None);
        assert!(graph.add_node(node(10, None)));
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
}
