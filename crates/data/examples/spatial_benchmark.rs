//! Warm viewport query benchmark; both paths materialize stable node IDs.
use gpug_data::{spatial::*, *};
use std::{hint::black_box, sync::Arc, time::Instant};

fn quantiles(mut values: Vec<u128>) -> [u128; 3] {
    values.sort_unstable();
    [
        values[values.len() / 2],
        values[values.len() * 95 / 100],
        values[values.len() * 99 / 100],
    ]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("workload,nodes,index_build_ms,index_bytes,result_nodes,examined_nodes,index_p50_ns,index_p95_ns,index_p99_ns,scan_p50_ns,scan_p95_ns,scan_p99_ns");
    for overlap in [false, true] {
        for n in [1_000u64, 10_000, 100_000, 1_000_000] {
            let graph = Arc::new(GraphSnapshot::new(
                (0..n)
                    .map(|id| {
                        Node::new(
                            id,
                            WorldRect {
                                x: if overlap {
                                    0.
                                } else {
                                    (id % 1000) as f64 * 10.
                                },
                                y: if overlap {
                                    0.
                                } else {
                                    (id / 1000) as f64 * 10.
                                },
                                width: 2.,
                                height: 2.,
                            },
                        )
                    })
                    .collect(),
                vec![],
            )?);
            let start = Instant::now();
            let index = SpatialIndex::new(Arc::clone(&graph));
            let build_ms = start.elapsed().as_millis();
            let mut workspace = SpatialWorkspace::new();
            let mut scan_ids = Vec::new();
            let budget = SpatialBudget {
                max_branches: usize::MAX,
                max_nodes: usize::MAX,
                max_results: usize::MAX,
            };
            let mut indexed = Vec::with_capacity(100);
            let mut linear = Vec::with_capacity(100);
            let mut result_count = 0;
            let mut examined = 0;
            for sample in 0..101 {
                let viewport = black_box(if overlap {
                    WorldRect {
                        x: -1.,
                        y: -1.,
                        width: 4.,
                        height: 4.,
                    }
                } else {
                    WorldRect {
                        x: (450 + sample) as f64 * 10.,
                        y: (n / 2000) as f64 * 10.,
                        width: 100.,
                        height: 100.,
                    }
                });
                let start = Instant::now();
                let result = workspace.query(black_box(&index), viewport, budget)?;
                black_box(&result);
                let indexed_ns = start.elapsed().as_nanos();
                assert!(!result.truncated);
                result_count = result.nodes.len();
                examined = result.examined_nodes;
                let start = Instant::now();
                scan_ids.clear();
                scan_ids.extend(
                    black_box(graph.nodes())
                        .iter()
                        .filter(|node| !node.hidden && node.bounds.intersects(viewport))
                        .map(|node| node.id),
                );
                black_box(&scan_ids);
                let scan_ns = start.elapsed().as_nanos();
                assert_eq!(result_count, scan_ids.len());
                if sample == 0 {
                    let mut ids = result.nodes.to_vec();
                    ids.sort_unstable();
                    assert_eq!(ids, scan_ids);
                } else {
                    indexed.push(indexed_ns);
                    linear.push(scan_ns);
                }
            }
            let [a, b, c] = quantiles(indexed);
            let [d, e, f] = quantiles(linear);
            let workload = if overlap { "overlap" } else { "grid" };
            println!(
                "{workload},{n},{build_ms},{},{result_count},{examined},{a},{b},{c},{d},{e},{f}",
                index.allocated_bytes()
            );
        }
    }
    Ok(())
}
