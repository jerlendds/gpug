//! Run with `cargo run -p gpug-data --release --example neighborhood_benchmark`.
//! Measures warm one-hop chain queries against a full edge-list scan baseline.
use gpug_data::{
    Direction, Edge, GraphSnapshot, Node, NodeId, SearchBudget, SearchWorkspace, WorldRect,
};
use std::hint::black_box;
use std::time::Instant;

fn quantiles(mut samples: Vec<u128>) -> [u128; 3] {
    samples.sort_unstable();
    [
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
        samples[samples.len() * 99 / 100],
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "nodes,edges,build_ms,csr_p50_ns,csr_p95_ns,csr_p99_ns,scan_p50_ns,scan_p95_ns,scan_p99_ns"
    );
    for n in [1_000u64, 10_000, 100_000, 1_000_000] {
        let start = Instant::now();
        let graph = GraphSnapshot::new(
            (0..n)
                .map(|id| Node::new(id, WorldRect::default()))
                .collect(),
            (0..n - 1).map(|id| Edge::new(id, id, id + 1)).collect(),
        )?;
        let build_ms = start.elapsed().as_millis();
        let mut workspace = SearchWorkspace::new();
        let budget = SearchBudget {
            max_hops: 1,
            max_nodes: 10,
            max_edges: 10,
        };
        workspace.neighborhood(&graph, NodeId(n / 2), Direction::Outgoing, budget)?;
        let mut csr = Vec::with_capacity(200);
        let mut scan = Vec::with_capacity(200);
        for sample in 0..200 {
            let seed = black_box(NodeId(n / 2 + sample));
            let start = Instant::now();
            let result =
                workspace.neighborhood(black_box(&graph), seed, Direction::Outgoing, budget)?;
            black_box(&result);
            csr.push(start.elapsed().as_nanos());
            assert_eq!(result.nodes.len(), 2);
            assert!(!result.truncated);
            let start = Instant::now();
            let count = black_box(graph.edges())
                .iter()
                .filter(|edge| edge.source == seed)
                .count();
            black_box(count);
            scan.push(start.elapsed().as_nanos());
            assert_eq!(count, 1);
        }
        let [a, b, c] = quantiles(csr);
        let [d, e, f] = quantiles(scan);
        println!("{n},{},{build_ms},{a},{b},{c},{d},{e},{f}", n - 1);
    }
    Ok(())
}
