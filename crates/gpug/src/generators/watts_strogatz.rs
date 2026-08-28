use crate::edge::Edge;
use crate::generators::utils::generate_nodes_with_seed;
use crate::generators::utils::rand_f32;
use crate::GraphData;

use std::collections::HashSet;

#[allow(clippy::needless_range_loop)]
pub fn generate_watts_strogatz_graph(n: usize, k: usize, beta: f32) -> Vec<Edge> {
    generate_watts_strogatz_graph_with_seed(n, k, beta, 0xBADC0FFE_E0DDF00D)
}

#[allow(clippy::needless_range_loop)]
pub fn generate_watts_strogatz_graph_with_seed(
    n: usize,
    k: usize,
    beta: f32,
    mut seed: u64,
) -> Vec<Edge> {
    if n < 2 {
        return Vec::new();
    }

    let max_k = (n - 1) / 2;
    let effective_k = k.min(max_k.max(1));
    if effective_k == 0 {
        return Vec::new();
    }

    let rewiring_prob = beta.clamp(0.0, 1.0);

    let mut adjacency: Vec<HashSet<usize>> = (0..n)
        .map(|_| HashSet::with_capacity(effective_k * 2))
        .collect();
    let mut targets: Vec<Vec<usize>> = (0..n).map(|_| Vec::with_capacity(effective_k)).collect();

    // Build the initial ring lattice connecting each node to its next `k` neighbors
    for source in 0..n {
        for offset in 1..=effective_k {
            let target = (source + offset) % n;
            if adjacency[source].insert(target) {
                adjacency[target].insert(source);
            }
            targets[source].push(target);
        }
    }

    if rewiring_prob > 0.0 {
        let attempt_cap = n * 8;
        for source in 0..n {
            let degree = targets[source].len();
            for edge_index in 0..degree {
                if rand_f32(&mut seed) >= rewiring_prob {
                    continue;
                }

                let old_target = targets[source][edge_index];
                adjacency[source].remove(&old_target);
                adjacency[old_target].remove(&source);

                let mut new_target = old_target;
                let mut attempts = 0usize;
                loop {
                    attempts += 1;
                    if attempts > attempt_cap {
                        adjacency[source].insert(old_target);
                        adjacency[old_target].insert(source);
                        break;
                    }
                    let mut candidate = (rand_f32(&mut seed) * n as f32) as usize;
                    if candidate >= n {
                        candidate = n - 1;
                    }
                    if candidate == source {
                        continue;
                    }
                    if adjacency[source].contains(&candidate) {
                        continue;
                    }
                    adjacency[source].insert(candidate);
                    adjacency[candidate].insert(source);
                    new_target = candidate;
                    break;
                }
                targets[source][edge_index] = new_target;
            }
        }
    }

    let mut edges = Vec::with_capacity(n * effective_k);
    for source in 0..n {
        for &target in &adjacency[source] {
            if source < target {
                edges.push(Edge::new(source, target));
            }
        }
    }
    edges
}

#[derive(Clone, Copy, Debug)]
pub struct WattsStrogatz {
    node_count: usize,
    neighbors: usize,
    rewiring_probability: f32,
    seed: u64,
}

impl WattsStrogatz {
    pub fn new(node_count: usize) -> Self {
        Self {
            node_count,
            neighbors: 3,
            rewiring_probability: 0.05,
            seed: 42,
        }
    }

    pub fn neighbors(mut self, neighbors: usize) -> Self {
        self.neighbors = neighbors;
        self
    }

    pub fn rewiring_probability(mut self, probability: f32) -> Self {
        self.rewiring_probability = probability;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn generate(self) -> GraphData {
        GraphData::new(
            generate_nodes_with_seed(self.node_count, self.seed ^ 0xCAFE_BABE),
            generate_watts_strogatz_graph_with_seed(
                self.node_count,
                self.neighbors,
                self.rewiring_probability,
                self.seed,
            ),
        )
    }
}
