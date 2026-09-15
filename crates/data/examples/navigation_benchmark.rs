//! Deterministic camera replay; CPU preparation only, not input-to-present latency.
//! Run in release mode. CSV includes the cold first frame of each source.
use gpug_data::{spatial::*, *};
use std::{hint::black_box, sync::Arc, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("workload,nodes,edges,event,frame,zoom,cache_hit,items,detailed,budget_aggregates,groups,plan_ns,connections_ns,total_ns");
    for workload in ["grid", "hub", "overlap"] {
        for count in [1_000u64, 10_000, 100_000, 1_000_000] {
            let side = (count as f64).sqrt().ceil() as u64;
            let source = Arc::new(GraphSnapshot::new(
                (0..count)
                    .map(|id| {
                        Node::new(
                            id,
                            WorldRect {
                                x: if workload == "overlap" {
                                    0.
                                } else {
                                    (id % side) as f64 * 10.
                                },
                                y: if workload == "overlap" {
                                    0.
                                } else {
                                    (id / side) as f64 * 10.
                                },
                                width: 2.,
                                height: 2.,
                            },
                        )
                    })
                    .collect(),
                (0..count)
                    .flat_map(|id| {
                        let mut edges = [
                            Edge::new(id * 2, id, (id + 1) % count),
                            Edge::new(
                                id * 2 + 1,
                                id,
                                if workload == "hub" {
                                    0
                                } else {
                                    (id + side) % count
                                },
                            ),
                        ];
                        // The ring's final edge also targets the hub. Model the
                        // parallel connection with an explicit distinct handle.
                        if edges[0].target == edges[1].target {
                            edges[1].source_handle = Some("secondary".into());
                        }
                        edges
                    })
                    .collect(),
            )?);
            let index = Arc::new(SpatialIndex::new(source));
            let mut lod = LodWorkspace::new();
            let mut cache = ConnectionCache::new();
            let extent = side as f64 * 10.;
            // Fixed 1000x700 screen. The first segment is a cache-hit control;
            // the next two change the viewport continuously, without resetting.
            for event in ["stationary", "pan", "zoom"] {
                for frame in 0..60 {
                    let progress = f64::from(frame) / 59.;
                    let overview = 700. / extent;
                    let zoom = if event == "zoom" {
                        overview * (60. / overview).powf(progress)
                    } else {
                        overview
                    };
                    let center = if workload == "overlap" {
                        1.
                    } else {
                        extent * 0.5
                    };
                    let offset = if event == "pan" {
                        progress * extent * 0.75
                    } else {
                        0.
                    };
                    let viewport = WorldRect {
                        x: center + offset - 500. / zoom,
                        y: center - 350. / zoom,
                        width: 1000. / zoom,
                        height: 700. / zoom,
                    };
                    let hits = cache.stats().hits;
                    let start = Instant::now();
                    let plan = Arc::new(lod.prepare(
                        Arc::clone(&index),
                        viewport,
                        LodOptions {
                            zoom,
                            max_zoom: 60.,
                            ..Default::default()
                        },
                    )?);
                    let plan_ns = start.elapsed().as_nanos();
                    let connection_start = Instant::now();
                    let connections =
                        cache.prepare(Arc::clone(&plan), ConnectionBudget::default(), || false)?;
                    black_box(&connections);
                    let connections_ns = connection_start.elapsed().as_nanos();
                    let total_ns = start.elapsed().as_nanos();
                    let hit = cache.stats().hits > hits;

                    // Validation is outside timing. Counts include internal and
                    // outside groups, so no edge silently vanishes during LOD.
                    assert_eq!(
                        connections
                            .groups()
                            .iter()
                            .map(|group| group.edge_count)
                            .sum::<usize>()
                            + connections.hidden_edges,
                        index.snapshot().edges().len(),
                    );
                    assert!(plan.items().len() <= LodOptions::default().max_items);
                    // At each segment's endpoints, independently scan all source
                    // rectangles and verify every visible source has an owner.
                    if frame == 0 || frame == 59 {
                        for node in index.snapshot().nodes() {
                            let b = node.bounds;
                            if b.x <= viewport.x + viewport.width
                                && b.x + b.width >= viewport.x
                                && b.y <= viewport.y + viewport.height
                                && b.y + b.height >= viewport.y
                            {
                                assert!(connections.node_item(node.id).is_some());
                            }
                        }
                    }
                    let detailed = plan
                        .items()
                        .iter()
                        .filter(|item| matches!(item.kind, LodKind::Node(_)))
                        .count();
                    println!(
                        "{workload},{count},{},{event},{frame},{zoom},{hit},{},{detailed},{},{},{plan_ns},{connections_ns},{total_ns}",
                        count * 2, plan.items().len(), plan.budget_aggregates, connections.groups().len(),
                    );
                }
            }
        }
    }
    Ok(())
}
