use std::collections::HashSet;

use crate::edge::Edge;
use crate::generators::erdos_renyi::generate_erdos_renyi_graph_with_seed;
use crate::generators::utils::rand_f32;
use crate::{GraphData, Node, WorldPoint};

/// Builds a connected ring lattice plus rare uniformly random long-range
/// shortcuts. This preserves local clustering while shortening global paths.
pub fn generate_small_world_graph_with_seed(
    node_count: usize,
    local_neighbors: usize,
    shortcut_probability: f64,
    seed: u64,
) -> Vec<Edge> {
    if node_count < 2 {
        return Vec::new();
    }
    let local_neighbors = local_neighbors.min((node_count - 1) / 2);
    let mut pairs = HashSet::with_capacity(node_count * local_neighbors + 1_024);
    let mut edges = Vec::with_capacity(node_count * local_neighbors + 1_024);

    for source in 0..node_count {
        for offset in 1..=local_neighbors {
            let target = (source + offset) % node_count;
            let pair = ordered_pair(source, target);
            if pairs.insert(pair) {
                edges.push(Edge::new_with_id(pair.0, pair.1, edges.len() as u64));
            }
        }
    }

    for edge in generate_erdos_renyi_graph_with_seed(
        node_count,
        shortcut_probability,
        seed ^ 0xA076_1D64_78BD_642F,
    ) {
        let pair = ordered_pair(edge.source.index(), edge.target.index());
        if pairs.insert(pair) {
            edges.push(Edge::new_with_id(pair.0, pair.1, edges.len() as u64));
        }
    }
    edges
}

fn ordered_pair(first: usize, second: usize) -> (usize, usize) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SmallWorld {
    node_count: usize,
    local_neighbors: usize,
    shortcut_probability: f64,
    seed: u64,
}

impl SmallWorld {
    pub fn new(node_count: usize) -> Self {
        Self {
            node_count,
            local_neighbors: 3,
            shortcut_probability: 0.00001,
            seed: 42,
        }
    }

    pub fn local_neighbors(mut self, neighbors: usize) -> Self {
        self.local_neighbors = neighbors;
        self
    }

    pub fn shortcut_probability(mut self, probability: f64) -> Self {
        self.shortcut_probability = if probability.is_finite() {
            probability.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn generate(self) -> GraphData {
        GraphData::new(
            circular_nodes(self.node_count, self.seed ^ 0xCAFE_BABE),
            generate_small_world_graph_with_seed(
                self.node_count,
                self.local_neighbors,
                self.shortcut_probability,
                self.seed,
            ),
        )
    }
}

fn circular_nodes(node_count: usize, mut seed: u64) -> Vec<Node> {
    let center = WorldPoint::new(650.0, 450.0);
    let base_radius = 350.0;
    (0..node_count)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / node_count.max(1) as f32;
            let radial_jitter = (rand_f32(&mut seed) - 0.5) * 24.0;
            let radius = base_radius + radial_jitter;
            Node::new(
                index,
                WorldPoint::new(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_ring_lattice_and_rare_shortcuts() {
        let edges = generate_small_world_graph_with_seed(10_000, 3, 0.00001, 42);
        assert!((30_350..30_650).contains(&edges.len()), "{}", edges.len());
        let pairs: HashSet<_> = edges
            .iter()
            .map(|edge| ordered_pair(edge.source.index(), edge.target.index()))
            .collect();
        assert_eq!(pairs.len(), edges.len());
    }

    #[test]
    fn seeded_generation_includes_stable_edge_ids() {
        let first = generate_small_world_graph_with_seed(100, 3, 0.01, 42);
        let second = generate_small_world_graph_with_seed(100, 3, 0.01, 42);
        assert_eq!(first, second);
    }
}
