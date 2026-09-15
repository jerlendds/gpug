//! Former aggregate-ID scan alone versus complete current one-aggregate conversion.
use gpug::analysis::{spatial::*, *};
use gpug::gpug_bridge::*;
use std::{hint::black_box, sync::Arc, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("nodes,old_id_scan_p95_ns,current_bridge_p95_ns");
    for count in [1_000u64, 10_000, 100_000, 1_000_000] {
        let graph = Arc::new(GraphSnapshot::new(
            (0..count)
                .map(|id| Node::new(id, WorldRect::default()))
                .collect(),
            vec![],
        )?);
        let index = Arc::new(SpatialIndex::new(Arc::clone(&graph)));
        let plan = Arc::new(LodWorkspace::new().prepare(
            index,
            WorldRect {
                x: -1.,
                y: -1.,
                width: 2.,
                height: 2.,
            },
            LodOptions {
                max_items: 1,
                ..Default::default()
            },
        )?);
        let connections = plan.connections(ConnectionBudget::default(), || false)?;
        let mut old_times = Vec::new();
        let mut new_times = Vec::new();
        for sample in 0..31 {
            // The old bridge started this search from zero on every conversion.
            let start = Instant::now();
            let mut candidate = black_box(0u64);
            while graph.node_index(candidate.into()).is_some() {
                candidate += 1;
            }
            black_box(candidate);
            let old = start.elapsed().as_nanos();
            assert_eq!(candidate, count);
            let start = Instant::now();
            let view = to_lod_graph_data(connections.clone(), "aggregate")?;
            black_box(&view);
            let new = start.elapsed().as_nanos();
            assert_eq!(view.data.nodes.len(), 1);
            assert!(graph.node_index(NodeId(view.data.nodes[0].id.0)).is_none());
            if sample > 0 {
                old_times.push(old);
                new_times.push(new);
            }
        }
        old_times.sort_unstable();
        new_times.sort_unstable();
        println!("{count},{},{}", old_times[28], new_times[28]);
    }
    Ok(())
}
