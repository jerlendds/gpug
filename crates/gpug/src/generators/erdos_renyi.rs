use crate::edge::Edge;
use crate::generators::utils::generate_nodes_with_seed;
use crate::generators::utils::rand_f64;
use crate::GraphData;

/// Generates a deterministic undirected G(n, p) graph in expected O(n + m).
pub fn generate_erdos_renyi_graph(n: usize, probability: f64) -> Vec<Edge> {
    generate_erdos_renyi_graph_with_seed(n, probability, 0xD1B5_4A32_D192_ED03)
}

pub fn generate_erdos_renyi_graph_with_seed(
    n: usize,
    probability: f64,
    mut seed: u64,
) -> Vec<Edge> {
    let probability = probability.clamp(0.0, 1.0);
    if n < 2 || probability == 0.0 {
        return Vec::new();
    }
    if probability == 1.0 {
        let mut edges = Vec::with_capacity(n.saturating_mul(n.saturating_sub(1)) / 2);
        for target in 1..n {
            for source in 0..target {
                edges.push(Edge::new(source, target));
            }
        }
        return edges;
    }

    let expected = probability * n as f64 * (n - 1) as f64 * 0.5;
    let mut edges = Vec::with_capacity(expected.ceil() as usize);
    let log_not_probability = (-probability).ln_1p();
    // Batagelj-Brandes geometric skips over the lower adjacency triangle.
    let mut target = 1usize;
    let mut source = -1isize;
    while target < n {
        let uniform = rand_f64(&mut seed);
        source += 1 + ((1.0 - uniform).ln() / log_not_probability).floor() as isize;
        while source >= target as isize && target < n {
            source -= target as isize;
            target += 1;
        }
        if target < n {
            edges.push(Edge::new(source as usize, target));
        }
    }
    edges
}

#[derive(Clone, Copy, Debug)]
pub struct ErdosRenyi {
    node_count: usize,
    probability: f64,
    seed: u64,
}

impl ErdosRenyi {
    pub fn new(node_count: usize) -> Self {
        Self {
            node_count,
            probability: 0.01,
            seed: 42,
        }
    }

    pub fn edge_probability(mut self, probability: f64) -> Self {
        self.probability = probability;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn generate(self) -> GraphData {
        GraphData::new(
            generate_nodes_with_seed(self.node_count, self.seed ^ 0xCAFE_BABE),
            generate_erdos_renyi_graph_with_seed(self.node_count, self.probability, self.seed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_probability_extremes() {
        assert!(generate_erdos_renyi_graph(10, 0.0).is_empty());
        assert_eq!(generate_erdos_renyi_graph(10, 1.0).len(), 45);
    }

    #[test]
    fn produces_valid_unique_edges_near_expectation() {
        let edges = generate_erdos_renyi_graph(1_000, 0.01);
        assert!((4_500..5_500).contains(&edges.len()), "{}", edges.len());
        assert!(edges.iter().all(|edge| edge.source < edge.target));
        let mut pairs: Vec<_> = edges.iter().map(|e| (e.source, e.target)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), edges.len());
    }
}
