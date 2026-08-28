# API overview

## Core data

- `NodeId`: stable domain identifier.
- `Node`: ID, world position, world size, and selection state.
- `Edge`: source and target `NodeId`.
- `GraphData`: nodes and edges plus topology validation.
- `GraphDataError`: duplicate-node and unknown-endpoint errors.
- `LayoutEdge`: dense internal-facing topology passed to layouts.

## Coordinates

- `LayoutPoint`: external-layout `f64` coordinate.
- `WorldPoint`, `WorldSize`, `WorldBounds`: renderer-independent geometry.
- `Viewport`: pan/zoom and reversible screen transforms.

## View and configuration

- `Graph`: GPUI view, interaction, layout control, and data access.
- `GraphBuilder`: validated construction and dependency configuration.
- `GraphStyle`: rendering and interaction appearance.
- `GraphRenderer`: rendering and level-of-detail policy.

## Layout

- `Layout`, `LayoutStatus`: incremental layout contract.
- `ForceAtlas2`: built-in scalable layout.
- `BatchLayout`, `BatchLayoutAdapter`: batch `f64` integration.
- `AnimatedBatchLayout`: smooth batch-result interpolation.
- `LayoutOptions`, `LayoutFit`: time budget and result normalization.

## Generators

- `ErdosRenyi`: seeded complete-data builder.
- `WattsStrogatz`: seeded small-world complete-data builder.
- `SmallWorld`: connected ring lattice plus rare independent long-range
  shortcuts.
- Free generation functions for nodes, Erdős–Rényi edges, and
  Watts–Strogatz edges.

All primary types are re-exported at the crate root, so applications normally
only need `use gpug::{...}` rather than internal module paths.
