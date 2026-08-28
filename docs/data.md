# Graph data and generators

`GraphData` owns domain-facing nodes and edges:

```rust
let data = GraphData::new(
    vec![
        Node::new(42_u64, WorldPoint::new(10.0, 20.0)),
        Node::new(9001_u64, WorldPoint::new(80.0, 50.0)),
    ],
    vec![Edge::new(42_u64, 9001_u64)],
);
```

`NodeId` values are stable domain IDs and do not need to be consecutive.
`GraphData::compile_edges` validates duplicate IDs and unknown endpoints, then
resolves IDs to dense `LayoutEdge` indices once for high-performance rendering
and layout. `GraphBuilder::build`, `Graph::set_data`, `add_node`, and `add_edge`
perform this validation and return `GraphDataError` on failure.

Read data through `Graph::data`, `nodes`, `edges`, or `node`.

## Generators

The ergonomic generator API returns complete `GraphData`:

```rust
let data = ErdosRenyi::new(10_000)
    .edge_probability(0.01)
    .seed(42)
    .generate();
```

The lower-level deterministic functions remain available:

- `generate_nodes`
- `generate_nodes_with_seed`
- `generate_erdos_renyi_graph`
- `generate_erdos_renyi_graph_with_seed`
- `generate_watts_strogatz_graph`

`WattsStrogatz::new(node_count)` provides the equivalent complete-data builder
with `neighbors`, `rewiring_probability`, and `seed` configuration.

For a connected lattice with independently sampled long-range shortcuts, use:

```rust
let data = SmallWorld::new(10_000)
    .local_neighbors(3)
    .shortcut_probability(0.00001)
    .seed(42)
    .generate();
```

This produces 30,000 local lattice edges plus roughly 500 shortcuts at 10,000
nodes, retaining local clustering while reducing long graph distances.

Graph generation is deliberately separate from `Graph`; generator-specific
parameters never appear in the view constructor.
