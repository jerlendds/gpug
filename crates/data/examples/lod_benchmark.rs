//! Node-plan CPU benchmark. Does not measure edges, GPUI installation or painting.
use gpug_data::{spatial::*, *};
use std::{hint::black_box, sync::Arc, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("workload,nodes,zoom,items,aggregates,represented_nodes,branches,node_tests,p50_ns,p95_ns,p99_ns");
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
            let index = SpatialIndex::new(graph);
            for zoom in [0.1, 2.] {
                let mut workspace = LodWorkspace::new();
                let options = LodOptions {
                    zoom,
                    max_zoom: 2.,
                    ..Default::default()
                };
                let mut times = Vec::with_capacity(1000);
                let mut stats = (0, 0, 0, 0, 0);
                for sample in 0..1001 {
                    let viewport = if overlap {
                        WorldRect {
                            x: -1.,
                            y: -1.,
                            width: 4.,
                            height: 4.,
                        }
                    } else {
                        WorldRect {
                            x: 4000. + (sample % 100) as f64,
                            y: (n / 2000) as f64 * 10.,
                            width: 1000.,
                            height: 1000.,
                        }
                    };
                    let start = Instant::now();
                    let plan = workspace.plan(black_box(&index), black_box(viewport), options)?;
                    black_box(&plan);
                    let elapsed = start.elapsed().as_nanos();
                    stats = (
                        plan.items().len(),
                        plan.items()
                            .iter()
                            .filter(|item| item.kind == LodKind::Aggregate)
                            .count(),
                        plan.items()
                            .iter()
                            .map(|item| item.represented_nodes)
                            .sum::<usize>(),
                        plan.visited_branches,
                        plan.examined_nodes,
                    );
                    assert!(stats.0 <= options.max_items);
                    assert!(stats.3 <= options.max_branches);
                    assert!(stats.4 <= options.max_nodes);
                    if sample != 0 {
                        times.push(elapsed);
                    }
                }
                times.sort_unstable();
                let workload = if overlap { "overlap" } else { "grid" };
                println!(
                    "{workload},{n},{zoom},{},{},{},{},{},{},{},{}",
                    stats.0, stats.1, stats.2, stats.3, stats.4, times[500], times[950], times[990]
                );
            }
        }
    }
    Ok(())
}
