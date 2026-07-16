//! Multi-link mesh topology graph (LoRa / BLE / Wi-Fi) with path-cost routing.
//!
//! Builds a directed weighted graph of peer interfaces and computes lowest-cost
//! paths via deterministic Dijkstra (BTreeMap-based, no floats).

use std::collections::{BTreeMap, BTreeSet};

/// Physical / logical link technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkType {
    LoRa,
    Ble,
    Wifi,
}

impl LinkType {
    /// Base path cost units (lower is better). Tuned for offline mesh:
    /// Wi-Fi preferred when available, then BLE, then long-range LoRa.
    pub fn base_cost(self) -> u32 {
        match self {
            LinkType::Wifi => 10,
            LinkType::Ble => 40,
            LinkType::LoRa => 100,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LinkType::LoRa => "lora",
            LinkType::Ble => "ble",
            LinkType::Wifi => "wifi",
        }
    }
}

/// Directed edge between two node ids on a specific medium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEdge {
    pub from: String,
    pub to: String,
    pub link: LinkType,
    /// Total edge cost (base + dynamic penalties); lower is better.
    pub cost: u32,
    /// Optional quality 0–255 (higher is better); used when recomputing cost.
    pub quality: u8,
}

impl LinkEdge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, link: LinkType, quality: u8) -> Self {
        let from = from.into();
        let to = to.into();
        let cost = compute_edge_cost(link, quality);
        Self {
            from,
            to,
            link,
            cost,
            quality,
        }
    }
}

/// cost = base + (255 - quality) scaled; quality 255 → base only.
pub fn compute_edge_cost(link: LinkType, quality: u8) -> u32 {
    let penalty = (255u32.saturating_sub(quality as u32)) / 4;
    link.base_cost().saturating_add(penalty)
}

/// Node in the island topology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub id: String,
    /// Back-compat simple adjacency list (ids only).
    pub connections: Vec<String>,
    pub battery_weight: u32,
}

impl GraphNode {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            connections: Vec::new(),
            battery_weight: 500,
        }
    }
}

/// Result of a path query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub nodes: Vec<String>,
    pub total_cost: u32,
    pub hops: Vec<LinkEdge>,
}

/// Multi-link topology graph for a disconnected mesh island.
#[derive(Debug, Clone, Default)]
pub struct TopologyGraph {
    nodes: BTreeMap<String, GraphNode>,
    /// Adjacency: from → list of edges.
    edges: BTreeMap<String, Vec<LinkEdge>>,
}

impl TopologyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_node(&mut self, node: GraphNode) {
        let id = node.id.clone();
        self.nodes.insert(id, node);
    }

    pub fn add_node(&mut self, id: impl Into<String>) {
        let id = id.into();
        self.nodes
            .entry(id.clone())
            .or_insert_with(|| GraphNode::new(id));
    }

    pub fn node_ids(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.values().map(|v| v.len()).sum()
    }

    /// Add a directed link. Creates endpoint nodes if missing.
    pub fn add_link(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        link: LinkType,
        quality: u8,
    ) {
        let from = from.into();
        let to = to.into();
        self.add_node(from.clone());
        self.add_node(to.clone());
        let edge = LinkEdge::new(from.clone(), to.clone(), link, quality);
        self.push_edge(edge);

        // Maintain simple connections list on the node.
        if let Some(n) = self.nodes.get_mut(&from) {
            if !n.connections.contains(&to) {
                n.connections.push(to);
                n.connections.sort();
            }
        }
    }

    /// Bidirectional convenience (symmetric cost/quality).
    pub fn add_bidirectional(
        &mut self,
        a: impl Into<String>,
        b: impl Into<String>,
        link: LinkType,
        quality: u8,
    ) {
        let a = a.into();
        let b = b.into();
        self.add_link(a.clone(), b.clone(), link, quality);
        self.add_link(b, a, link, quality);
    }

    fn push_edge(&mut self, edge: LinkEdge) {
        let list = self.edges.entry(edge.from.clone()).or_default();
        // Replace existing same (to, link) if present.
        if let Some(pos) = list
            .iter()
            .position(|e| e.to == edge.to && e.link == edge.link)
        {
            list[pos] = edge;
        } else {
            list.push(edge);
        }
        list.sort_by(|a, b| a.to.cmp(&b.to).then_with(|| a.link.cmp(&b.link)));
    }

    pub fn links_from(&self, id: &str) -> &[LinkEdge] {
        self.edges
            .get(id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Lowest-cost path (Dijkstra). Deterministic: equal costs prefer
    /// lexicographically smaller next-hop id, then lower LinkType ord.
    pub fn shortest_path(&self, src: &str, dst: &str) -> Option<Path> {
        if src == dst {
            return Some(Path {
                nodes: vec![src.to_string()],
                total_cost: 0,
                hops: vec![],
            });
        }
        if !self.nodes.contains_key(src) || !self.nodes.contains_key(dst) {
            return None;
        }

        // dist[node] = best cost
        let mut dist: BTreeMap<String, u32> = BTreeMap::new();
        let mut prev: BTreeMap<String, (String, LinkEdge)> = BTreeMap::new();
        let mut visited: BTreeSet<String> = BTreeSet::new();

        dist.insert(src.to_string(), 0);

        loop {
            // Pick unvisited node with smallest dist; tie → smaller id.
            let mut current: Option<(String, u32)> = None;
            for (id, &d) in &dist {
                if visited.contains(id) {
                    continue;
                }
                match &current {
                    None => current = Some((id.clone(), d)),
                    Some((_, cd)) if d < *cd => current = Some((id.clone(), d)),
                    Some((cid, cd)) if d == *cd && id < cid => {
                        current = Some((id.clone(), d));
                    }
                    _ => {}
                }
            }
            let (u, u_cost) = match current {
                Some(x) => x,
                None => break,
            };
            if u == dst {
                break;
            }
            visited.insert(u.clone());

            let edges = match self.edges.get(&u) {
                Some(e) => e.clone(),
                None => continue,
            };

            // Stable edge order for equal-cost choices.
            let mut ordered = edges;
            ordered.sort_by(|a, b| {
                a.cost
                    .cmp(&b.cost)
                    .then_with(|| a.to.cmp(&b.to))
                    .then_with(|| a.link.cmp(&b.link))
            });

            for edge in ordered {
                if visited.contains(&edge.to) {
                    continue;
                }
                let alt = u_cost.saturating_add(edge.cost);
                let improve = match dist.get(&edge.to) {
                    None => true,
                    Some(&old) if alt < old => true,
                    Some(&old) if alt == old => {
                        // Prefer lexicographically better predecessor path edge
                        match prev.get(&edge.to) {
                            Some((p, e)) => {
                                edge.from < *p
                                    || (edge.from == *p
                                        && (edge.to < e.to
                                            || (edge.to == e.to && edge.link < e.link)))
                            }
                            None => true,
                        }
                    }
                    _ => false,
                };
                if improve {
                    dist.insert(edge.to.clone(), alt);
                    prev.insert(edge.to.clone(), (u.clone(), edge));
                }
            }
        }

        let total_cost = *dist.get(dst)?;
        // Reconstruct
        let mut hops = Vec::new();
        let mut nodes = vec![dst.to_string()];
        let mut cur = dst.to_string();
        while cur != src {
            let (p, edge) = prev.get(&cur)?.clone();
            hops.push(edge);
            nodes.push(p.clone());
            cur = p;
        }
        nodes.reverse();
        hops.reverse();
        Some(Path {
            nodes,
            total_cost,
            hops,
        })
    }

    /// Best next hop toward `dst` from `src`, if any path exists.
    pub fn next_hop(&self, src: &str, dst: &str) -> Option<(String, LinkType, u32)> {
        let path = self.shortest_path(src, dst)?;
        if path.hops.is_empty() {
            return None;
        }
        let h = &path.hops[0];
        Some((h.to.clone(), h.link, path.total_cost))
    }

    /// Export a simple adjacency summary for debugging.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for id in self.nodes.keys() {
            let links: Vec<String> = self
                .links_from(id)
                .iter()
                .map(|e| format!("{}:{}:{}", e.to, e.link.as_str(), e.cost))
                .collect();
            lines.push(format!("{} -> [{}]", id, links.join(", ")));
        }
        lines.join("\n")
    }
}

pub fn init_topology() {
    println!("Initializing Dynamic Topology Mapping for the local mesh island.");
    let mut g = TopologyGraph::new();
    g.add_bidirectional("phone", "pi", LinkType::Wifi, 240);
    g.add_bidirectional("pi", "esp32", LinkType::LoRa, 180);
    g.add_bidirectional("phone", "watch", LinkType::Ble, 200);
    g.add_bidirectional("watch", "esp32", LinkType::Ble, 150);
    if let Some(path) = g.shortest_path("phone", "esp32") {
        println!(
            "Topology path phone→esp32 cost={} via {:?}",
            path.total_cost, path.nodes
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_preferred_over_lora_direct() {
        let mut g = TopologyGraph::new();
        g.add_bidirectional("A", "B", LinkType::LoRa, 255);
        g.add_bidirectional("A", "B", LinkType::Wifi, 255);
        let path = g.shortest_path("A", "B").unwrap();
        assert_eq!(path.hops.len(), 1);
        assert_eq!(path.hops[0].link, LinkType::Wifi);
        assert_eq!(path.total_cost, LinkType::Wifi.base_cost());
    }

    #[test]
    fn multi_hop_cheaper_than_bad_direct() {
        let mut g = TopologyGraph::new();
        // Direct LoRa with terrible quality
        g.add_link("A", "C", LinkType::LoRa, 0);
        // A --wifi--> B --wifi--> C both excellent
        g.add_link("A", "B", LinkType::Wifi, 255);
        g.add_link("B", "C", LinkType::Wifi, 255);
        let path = g.shortest_path("A", "C").unwrap();
        assert_eq!(path.nodes, vec!["A", "B", "C"]);
        assert_eq!(path.total_cost, 20);
    }

    #[test]
    fn path_cost_deterministic() {
        let mut g = TopologyGraph::new();
        g.add_bidirectional("phone", "pi", LinkType::Wifi, 240);
        g.add_bidirectional("pi", "esp32", LinkType::LoRa, 180);
        g.add_bidirectional("phone", "watch", LinkType::Ble, 200);
        g.add_bidirectional("watch", "esp32", LinkType::Ble, 150);
        let p1 = g.shortest_path("phone", "esp32").unwrap();
        let p2 = g.shortest_path("phone", "esp32").unwrap();
        assert_eq!(p1, p2);
        assert!(!p1.nodes.is_empty());
        assert_eq!(p1.nodes[0], "phone");
        assert_eq!(*p1.nodes.last().unwrap(), "esp32");
    }

    #[test]
    fn no_path_returns_none() {
        let mut g = TopologyGraph::new();
        g.add_node("A");
        g.add_node("B");
        assert!(g.shortest_path("A", "B").is_none());
    }

    #[test]
    fn next_hop_reports_link_type() {
        let mut g = TopologyGraph::new();
        g.add_link("A", "B", LinkType::Ble, 200);
        g.add_link("B", "C", LinkType::LoRa, 200);
        let (nh, link, cost) = g.next_hop("A", "C").unwrap();
        assert_eq!(nh, "B");
        assert_eq!(link, LinkType::Ble);
        assert!(cost > 0);
    }

    #[test]
    fn graph_node_connections_updated() {
        let mut g = TopologyGraph::new();
        g.add_link("X", "Y", LinkType::Wifi, 255);
        g.add_link("X", "Z", LinkType::Ble, 255);
        let n = g.nodes.get("X").unwrap();
        assert_eq!(n.connections, vec!["Y".to_string(), "Z".to_string()]);
    }

    #[test]
    fn equal_cost_tie_is_stable() {
        let mut g = TopologyGraph::new();
        g.add_link("S", "A", LinkType::Wifi, 255);
        g.add_link("S", "B", LinkType::Wifi, 255);
        g.add_link("A", "T", LinkType::Wifi, 255);
        g.add_link("B", "T", LinkType::Wifi, 255);
        let p = g.shortest_path("S", "T").unwrap();
        // Both paths cost 20; Dijkstra picks deterministic first hop ("A" < "B")
        assert_eq!(p.nodes, vec!["S", "A", "T"]);
        assert_eq!(p.total_cost, 20);
    }
}
