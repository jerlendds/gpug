//! Whole-source connection planning cost, including temporary result destruction.
use gpug_data::{spatial::*, *};
use std::{hint::black_box, sync::Arc, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("workload,nodes,edges,zoom,items,groups,p50_ns,p95_ns,p99_ns,cache_p50_ns,cache_p95_ns,cache_p99_ns");
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
                (0..n)
                    .flat_map(|id| {
                        [
                            Edge::new(id * 2, id, (id + 1) % n),
                            Edge::new(id * 2 + 1, id, (id + 17) % n),
                        ]
                    })
                    .collect(),
            )?);
            let index = Arc::new(SpatialIndex::new(graph));
            for zoom in [0.1, 2.] {
                let viewport = if overlap {
                    WorldRect {
                        x: -1.,
                        y: -1.,
                        width: 4.,
                        height: 4.,
                    }
                } else {
                    WorldRect {
                        x: 4000.,
                        y: (n / 2000) as f64 * 10.,
                        width: 1000.,
                        height: 1000.,
                    }
                };
                let plan = Arc::new(LodWorkspace::new().prepare(
                    Arc::clone(&index),
                    viewport,
                    LodOptions {
                        zoom,
                        max_zoom: 2.,
                        ..Default::default()
                    },
                )?);
                let mut times = Vec::new();
                let mut cache_times = Vec::new();
                let mut cache = ConnectionCache::new();
                let mut groups = 0;
                for sample in 0..31 {
                    let start = Instant::now();
                    let summary = plan.connections(ConnectionBudget::default(), || false)?;
                    black_box(&summary);
                    groups = summary.groups().len();
                    let edge_count = summary
                        .groups()
                        .iter()
                        .map(|group| group.edge_count)
                        .sum::<usize>()
                        + summary.hidden_edges;
                    drop(summary);
                    let elapsed = start.elapsed().as_nanos();
                    assert_eq!(edge_count, (n * 2) as usize);
                    let start = Instant::now();
                    let cached =
                        cache.prepare(Arc::clone(&plan), ConnectionBudget::default(), || false)?;
                    black_box(&cached);
                    assert_eq!(cached.groups().len(), groups);
                    drop(cached);
                    let cache_elapsed = start.elapsed().as_nanos();
                    if sample > 0 {
                        times.push(elapsed);
                        cache_times.push(cache_elapsed);
                    }
                }
                times.sort_unstable();
                cache_times.sort_unstable();
                assert_eq!(cache.stats().builds, 1);
                assert_eq!(cache.stats().hits, 30);
                let workload = if overlap { "overlap" } else { "grid" };
                println!(
                    "{workload},{n},{},{zoom},{},{groups},{},{},{},{},{},{}",
                    n * 2,
                    plan.items().len(),
                    times[15],
                    times[28],
                    times[29],
                    cache_times[15],
                    cache_times[28],
                    cache_times[29]
                );
            }
        }
    }
    Ok(())
}
