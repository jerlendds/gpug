//! Focused node-plan CPU preparation; excludes connectivity, GUI and index construction.
use gpug_data::{spatial::*, *};
use std::{hint::black_box, sync::Arc, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("nodes,max_items,index_bytes,p50_ns,p95_ns,p99_ns");
    for count in [1_000u64, 10_000, 100_000, 1_000_000] {
        let index = Arc::new(SpatialIndex::new(Arc::new(GraphSnapshot::new(
            (0..count)
                .map(|id| {
                    Node::new(
                        id,
                        WorldRect {
                            x: 0.,
                            y: 0.,
                            width: 2.,
                            height: 2.,
                        },
                    )
                })
                .collect(),
            vec![],
        )?)));
        for max_items in [3, 512] {
            let mut workspace = LodWorkspace::new();
            let mut samples = Vec::with_capacity(1000);
            let options = LodOptions {
                zoom: 60.,
                max_items,
                ..Default::default()
            };
            for sample in 0..1001u64 {
                let focus = NodeId(sample * (count - 1) / 1000);
                let start = Instant::now();
                let plan = workspace.prepare_focused(
                    Arc::clone(&index),
                    WorldRect {
                        x: -1.,
                        y: -1.,
                        width: 4.,
                        height: 4.,
                    },
                    options,
                    Some(black_box(focus)),
                )?;
                black_box(&plan);
                let elapsed = start.elapsed().as_nanos();
                assert_eq!(plan.items().last().unwrap().kind, LodKind::Node(focus));
                assert_eq!(
                    plan.items()
                        .iter()
                        .map(|item| item.represented_nodes)
                        .sum::<usize>(),
                    count as usize
                );
                assert!(plan.items().len() <= max_items);
                assert!(plan.visited_branches <= options.max_branches);
                assert!(plan.examined_nodes <= options.max_nodes);
                if sample > 0 {
                    samples.push(elapsed);
                }
            }
            samples.sort_unstable();
            println!(
                "{count},{max_items},{},{},{},{}",
                index.allocated_bytes(),
                samples[499],
                samples[949],
                samples[989]
            );
        }
    }
    Ok(())
}
