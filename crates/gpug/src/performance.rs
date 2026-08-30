//! Incremental invalidation, revision tracking, geometry caching, and culling.
use crate::editor::NodeColumns;
use crate::{Edge, EdgeId, NodeId, WorldBounds};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Revisions {
    pub membership: u64,
    /// Advances whenever a node specification changes, independently of
    /// runtime motion and membership.
    pub node_specs: u64,
    /// Advances whenever an edge specification changes, independently of
    /// geometry invalidation and membership.
    pub edge_specs: u64,
    /// Advances whenever any node runtime changes, so consumers can stamp
    /// caches derived from node positions without diffing the dirty set.
    pub nodes: u64,
    /// Advances whenever any edge is invalidated.
    pub edges: u64,
    pub viewport: u64,
    pub selection: u64,
    pub connection: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DirtySet {
    pub nodes: HashSet<NodeId>,
    pub edges: HashSet<EdgeId>,
    pub overlays: bool,
    pub viewport: bool,
    pub membership: bool,
    /// Set when every node and edge moved at once, as during a layout frame.
    /// Per-entity sets stay empty because enumerating them costs more than the
    /// consumer saves; treat `all` as "assume everything changed".
    pub all: bool,
}

#[derive(Default)]
pub struct DirtyTracker {
    pub revisions: Revisions,
    dirty: DirtySet,
    adjacency: HashMap<NodeId, Vec<EdgeId>>,
}

impl DirtyTracker {
    pub fn rebuild(&mut self, edges: &[Edge]) {
        self.adjacency.clear();
        for edge in edges {
            self.adjacency.entry(edge.source).or_default().push(edge.id);
            self.adjacency.entry(edge.target).or_default().push(edge.id);
        }
        self.revisions.membership = self.revisions.membership.wrapping_add(1);
        self.dirty.membership = true;
    }
    pub fn mark_node(&mut self, id: NodeId) {
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        self.dirty.nodes.insert(id);
        if let Some(edges) = self.adjacency.get(&id) {
            self.revisions.edges = self.revisions.edges.wrapping_add(1);
            self.dirty.edges.extend(edges.iter().copied());
        }
    }
    pub fn mark_node_spec(&mut self) {
        self.revisions.node_specs = self.revisions.node_specs.wrapping_add(1);
    }
    pub fn mark_edge_spec(&mut self) {
        self.revisions.edge_specs = self.revisions.edge_specs.wrapping_add(1);
    }
    pub fn mark_edge(&mut self, id: EdgeId) {
        self.revisions.edges = self.revisions.edges.wrapping_add(1);
        self.dirty.edges.insert(id);
    }
    /// Invalidates every node and edge in one step. A layout frame moves the
    /// whole graph, so per-node adjacency propagation would insert every edge
    /// id into the dirty set each frame while telling consumers nothing they
    /// could not infer from `all`.
    pub fn mark_all(&mut self) {
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        self.revisions.edges = self.revisions.edges.wrapping_add(1);
        self.dirty.all = true;
    }
    pub fn mark_viewport(&mut self) {
        self.revisions.viewport = self.revisions.viewport.wrapping_add(1);
        self.dirty.viewport = true;
    }
    pub fn mark_selection(&mut self) {
        self.revisions.selection = self.revisions.selection.wrapping_add(1);
        self.dirty.overlays = true;
    }
    pub fn mark_connection(&mut self) {
        self.revisions.connection = self.revisions.connection.wrapping_add(1);
        self.dirty.overlays = true;
    }
    pub fn take(&mut self) -> DirtySet {
        std::mem::take(&mut self.dirty)
    }
    pub fn peek(&self) -> &DirtySet {
        &self.dirty
    }
}

#[derive(Clone, Debug)]
struct CacheEntry<G> {
    edge_revision: u64,
    source_revision: u64,
    target_revision: u64,
    geometry: G,
}
#[derive(Clone, Debug, Default)]
pub struct GeometryCache<G> {
    entries: HashMap<EdgeId, CacheEntry<G>>,
}
impl<G: Clone> GeometryCache<G> {
    pub fn get_or_insert_with(
        &mut self,
        id: EdgeId,
        stamp: (u64, u64, u64),
        compute: impl FnOnce() -> G,
    ) -> G {
        if let Some(entry) = self.entries.get(&id) {
            if (
                entry.edge_revision,
                entry.source_revision,
                entry.target_revision,
            ) == stamp
            {
                return entry.geometry.clone();
            }
        }
        let geometry = compute();
        self.entries.insert(
            id,
            CacheEntry {
                edge_revision: stamp.0,
                source_revision: stamp.1,
                target_revision: stamp.2,
                geometry: geometry.clone(),
            },
        );
        geometry
    }
    pub fn invalidate(&mut self, id: EdgeId) {
        self.entries.remove(&id);
    }
    pub fn retain(&mut self, ids: &HashSet<EdgeId>) {
        self.entries.retain(|id, _| ids.contains(id));
    }
}

/// Spatial index over node bounds, keyed by dense node index.
///
/// Bounds live in four `f32` columns rather than one rectangle per node, so a
/// rebuild or a full scan is a sequential pass over contiguous memory that the
/// prefetcher and the auto-vectorizer can both follow.
///
/// On top of the columns sits a uniform grid. Nodes are bucketed into
/// fixed-size cells by a counting sort, and a camera query visits only the
/// cells the camera rectangle overlaps, so off-screen regions are rejected in
/// bulk instead of one node at a time.
///
/// A uniform grid suits this workload specifically because a force-directed
/// frame moves every node at once. Rebuilding is two linear counting passes
/// with no allocation and no pointer chasing, where a quadtree, BVH, or R-tree
/// would pay for node allocation and rebalancing every frame to answer the
/// same query. The grid is rebuilt lazily: a frame that only pans the camera
/// reuses the buckets from the frame that last moved a node.
#[derive(Clone, Debug, Default)]
pub struct VisibilityIndex {
    grid: Grid,
}

/// Uniform grid bucket table. `starts` is a prefix sum over cell occupancy and
/// `items` holds node indices grouped by cell, so a cell's members are one
/// contiguous slice.
#[derive(Clone, Debug, Default)]
struct Grid {
    built_revision: u64,
    built: bool,
    origin_x: f32,
    origin_y: f32,
    cell_size: f32,
    columns: usize,
    rows: usize,
    /// Largest node extent in cells, used to widen a query so that a node
    /// bucketed by its minimum corner is still found from a neighbouring cell.
    halo: usize,
    starts: Vec<u32>,
    items: Vec<u32>,
    cursors: Vec<u32>,
}

impl VisibilityIndex {
    /// Culls `columns` against `camera`, writing the surviving node indices in
    /// ascending order into `visible` and a per-index flag into `flags`.
    ///
    /// `flags` is resized to the column length and fully overwritten, so an
    /// edge can test both endpoints with two array reads instead of two set
    /// lookups.
    pub fn cull(
        &mut self,
        columns: &NodeColumns,
        camera: WorldBounds,
        visible: &mut Vec<u32>,
        flags: &mut Vec<bool>,
    ) {
        let count = columns.len();
        visible.clear();
        flags.clear();
        flags.resize(count, false);
        if count == 0 {
            return;
        }
        let camera_max_x = camera.origin.x + camera.size.width;
        let camera_max_y = camera.origin.y + camera.size.height;

        // Below this size the grid's bookkeeping costs more than the linear
        // scan it replaces: the four column reads per node are sequential and
        // vectorize well.
        const GRID_THRESHOLD: usize = 1_024;
        if count < GRID_THRESHOLD {
            for index in 0..count {
                if columns.x[index] + columns.width[index] >= camera.origin.x
                    && columns.x[index] <= camera_max_x
                    && columns.y[index] + columns.height[index] >= camera.origin.y
                    && columns.y[index] <= camera_max_y
                {
                    visible.push(index as u32);
                    flags[index] = true;
                }
            }
            return;
        }

        self.build_grid(columns);
        let grid = &self.grid;
        if !grid.built {
            for index in 0..count {
                visible.push(index as u32);
                flags[index] = true;
            }
            return;
        }
        let first_column = grid.column_of(camera.origin.x).saturating_sub(grid.halo);
        let last_column = (grid.column_of(camera_max_x) + grid.halo).min(grid.columns - 1);
        let first_row = grid.row_of(camera.origin.y).saturating_sub(grid.halo);
        let last_row = (grid.row_of(camera_max_y) + grid.halo).min(grid.rows - 1);
        for row in first_row..=last_row {
            let row_base = row * grid.columns;
            for column in first_column..=last_column {
                let cell = row_base + column;
                let start = grid.starts[cell] as usize;
                let end = grid.starts[cell + 1] as usize;
                for &item in &grid.items[start..end] {
                    let index = item as usize;
                    if columns.x[index] + columns.width[index] >= camera.origin.x
                        && columns.x[index] <= camera_max_x
                        && columns.y[index] + columns.height[index] >= camera.origin.y
                        && columns.y[index] <= camera_max_y
                    {
                        flags[index] = true;
                    }
                }
            }
        }
        // Ascending order lets consumers walk the scene columns forwards.
        for (index, visible_flag) in flags.iter().enumerate() {
            if *visible_flag {
                visible.push(index as u32);
            }
        }
    }

    /// Buckets every node into a cell. Two linear counting passes, no
    /// allocation after the first frame, and no work at all when the columns
    /// have not been written since the last build.
    fn build_grid(&mut self, columns: &NodeColumns) {
        if self.grid.built && self.grid.built_revision == columns.revision() {
            return;
        }
        let count = columns.len();
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        let (mut extent_x, mut extent_y) = (0.0f32, 0.0f32);
        for index in 0..count {
            min_x = min_x.min(columns.x[index]);
            min_y = min_y.min(columns.y[index]);
            max_x = max_x.max(columns.x[index] + columns.width[index]);
            max_y = max_y.max(columns.y[index] + columns.height[index]);
            extent_x = extent_x.max(columns.width[index]);
            extent_y = extent_y.max(columns.height[index]);
        }
        let grid = &mut self.grid;
        grid.built = false;
        if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
            return;
        }
        // Roughly two nodes per cell keeps the bucket table small enough to
        // stay in cache while still rejecting most of the graph per query.
        let dimension = ((count as f32 / 2.0).sqrt().ceil() as usize).clamp(1, 256);
        let span = (max_x - min_x).max(max_y - min_y).max(1.0);
        let cell_size = (span / dimension as f32).max(f32::MIN_POSITIVE);
        grid.origin_x = min_x;
        grid.origin_y = min_y;
        grid.cell_size = cell_size;
        grid.columns = (((max_x - min_x) / cell_size).ceil() as usize + 1).clamp(1, 4_096);
        grid.rows = (((max_y - min_y) / cell_size).ceil() as usize + 1).clamp(1, 4_096);
        grid.halo = ((extent_x.max(extent_y) / cell_size).ceil() as usize).min(64);

        let cells = grid.columns * grid.rows;
        grid.starts.clear();
        grid.starts.resize(cells + 1, 0);
        for index in 0..count {
            let cell = grid.cell_of(columns.x[index], columns.y[index]);
            grid.starts[cell + 1] += 1;
        }
        for cell in 1..=cells {
            grid.starts[cell] += grid.starts[cell - 1];
        }
        grid.cursors.clear();
        grid.cursors.extend_from_slice(&grid.starts[..cells]);
        grid.items.clear();
        grid.items.resize(count, 0);
        for index in 0..count {
            let cell = grid.cell_of(columns.x[index], columns.y[index]);
            let cursor = &mut grid.cursors[cell];
            grid.items[*cursor as usize] = index as u32;
            *cursor += 1;
        }
        grid.built = true;
        grid.built_revision = columns.revision();
    }

    /// Node indices whose bounds intersect `rect`, appended to `out` in
    /// ascending order.
    ///
    /// This is the pointer hot path. A mouse move at 120 Hz must not scan the
    /// graph, so a query visits the cells the rectangle covers and tests only
    /// what is bucketed there.
    ///
    /// It takes `&self` deliberately: hit testing runs from event handlers
    /// that hold the graph immutably, and rebuilding an index from one would
    /// be a write on the input path. The grid is built during the frame's
    /// cull instead; a query that arrives before the first cull, or after the
    /// columns moved, falls back to the linear scan rather than going stale.
    pub fn query(&self, columns: &NodeColumns, rect: WorldBounds, out: &mut Vec<u32>) {
        out.clear();
        let count = columns.len();
        if count == 0 {
            return;
        }
        let max_x = rect.origin.x + rect.size.width;
        let max_y = rect.origin.y + rect.size.height;
        let intersects = |index: usize| {
            columns.x[index] + columns.width[index] >= rect.origin.x
                && columns.x[index] <= max_x
                && columns.y[index] + columns.height[index] >= rect.origin.y
                && columns.y[index] <= max_y
        };
        let grid = &self.grid;
        if !grid.built || grid.built_revision != columns.revision() {
            out.extend(
                (0..count)
                    .filter(|index| intersects(*index))
                    .map(|index| index as u32),
            );
            return;
        }
        let first_column = grid.column_of(rect.origin.x).saturating_sub(grid.halo);
        let last_column = (grid.column_of(max_x) + grid.halo).min(grid.columns - 1);
        let first_row = grid.row_of(rect.origin.y).saturating_sub(grid.halo);
        let last_row = (grid.row_of(max_y) + grid.halo).min(grid.rows - 1);
        for row in first_row..=last_row {
            let row_base = row * grid.columns;
            for column in first_column..=last_column {
                let cell = row_base + column;
                let start = grid.starts[cell] as usize;
                let end = grid.starts[cell + 1] as usize;
                for &item in &grid.items[start..end] {
                    if intersects(item as usize) {
                        out.push(item);
                    }
                }
            }
        }
        out.sort_unstable();
    }
}

impl Grid {
    #[inline]
    fn column_of(&self, x: f32) -> usize {
        (((x - self.origin_x) / self.cell_size).floor().max(0.0) as usize).min(self.columns - 1)
    }
    #[inline]
    fn row_of(&self, y: f32) -> usize {
        (((y - self.origin_y) / self.cell_size).floor().max(0.0) as usize).min(self.rows - 1)
    }
    #[inline]
    fn cell_of(&self, x: f32, y: f32) -> usize {
        self.row_of(y) * self.columns + self.column_of(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::NO_PARENT;
    use crate::{Edge, Node, NodeId, WorldPoint, WorldSize};

    fn columns(bounds: &[(f32, f32, f32, f32)]) -> NodeColumns {
        let mut columns = NodeColumns::default();
        for (index, (x, y, width, height)) in bounds.iter().enumerate() {
            let node = Node::new(index as u64 + 1, WorldPoint::new(*x, *y))
                .with_size(WorldSize::new(*width, *height));
            columns.push_for_test(&node, WorldPoint::new(*x, *y), WorldSize::new(*width, *height), NO_PARENT);
        }
        columns.touch();
        columns
    }

    fn brute_force(columns: &NodeColumns, camera: WorldBounds) -> Vec<u32> {
        let max_x = camera.origin.x + camera.size.width;
        let max_y = camera.origin.y + camera.size.height;
        (0..columns.len())
            .filter(|index| {
                columns.x[*index] + columns.width[*index] >= camera.origin.x
                    && columns.x[*index] <= max_x
                    && columns.y[*index] + columns.height[*index] >= camera.origin.y
                    && columns.y[*index] <= max_y
            })
            .map(|index| index as u32)
            .collect()
    }

    /// The grid exists to avoid touching every node. It is only worth having if
    /// it returns exactly what the scan it replaces would have returned, so
    /// that is what this checks - at a size above the threshold where the grid
    /// actually engages, and with nodes of mixed extents so the halo matters.
    #[test]
    fn grid_cull_matches_a_linear_scan() {
        let mut bounds = Vec::new();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for index in 0..4_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let x = (seed % 10_000) as f32 - 5_000.0;
            let y = ((seed >> 20) % 10_000) as f32 - 5_000.0;
            // Every 50th node is large enough to span several cells.
            let extent = if index % 50 == 0 { 400.0 } else { 6.0 };
            bounds.push((x, y, extent, extent));
        }
        let columns = columns(&bounds);
        let mut index = VisibilityIndex::default();
        let (mut visible, mut flags) = (Vec::new(), Vec::new());
        for camera in [
            WorldBounds::new(WorldPoint::new(-100.0, -100.0), WorldSize::new(200.0, 200.0)),
            WorldBounds::new(WorldPoint::new(2_000.0, -4_000.0), WorldSize::new(900.0, 500.0)),
            WorldBounds::new(WorldPoint::new(-9_000.0, -9_000.0), WorldSize::new(50.0, 50.0)),
            WorldBounds::new(
                WorldPoint::new(-6_000.0, -6_000.0),
                WorldSize::new(12_000.0, 12_000.0),
            ),
        ] {
            index.cull(&columns, camera, &mut visible, &mut flags);
            assert_eq!(visible, brute_force(&columns, camera), "camera {camera:?}");
            assert!(visible.iter().all(|node| flags[*node as usize]));
            assert_eq!(
                flags.iter().filter(|flag| **flag).count(),
                visible.len(),
                "flags and list must agree"
            );
        }
    }

    #[test]
    fn a_query_finds_every_node_under_a_point() {
        let mut bounds = Vec::new();
        for index in 0..2_000 {
            bounds.push(((index % 40) as f32 * 25.0, (index / 40) as f32 * 25.0, 10.0, 10.0));
        }
        // Two overlapping nodes sit on the same spot as node 0.
        bounds.push((0.0, 0.0, 10.0, 10.0));
        let columns = columns(&bounds);
        let mut index = VisibilityIndex::default();
        let (mut visible, mut flags) = (Vec::new(), Vec::new());
        index.cull(
            &columns,
            WorldBounds::new(
                WorldPoint::new(-10_000.0, -10_000.0),
                WorldSize::new(20_000.0, 20_000.0),
            ),
            &mut visible,
            &mut flags,
        );

        let mut hits = Vec::new();
        index.query(
            &columns,
            WorldBounds::new(WorldPoint::new(5.0, 5.0), WorldSize::new(0.0, 0.0)),
            &mut hits,
        );
        assert_eq!(hits, vec![0, 2_000]);
    }

    /// A query that arrives before the grid has been built, or after the
    /// columns moved, must still be correct rather than stale.
    #[test]
    fn a_query_falls_back_to_a_scan_when_the_grid_is_stale() {
        let mut bounds = vec![(0.0, 0.0, 10.0, 10.0)];
        for index in 1..2_000 {
            bounds.push((index as f32 * 40.0, 0.0, 10.0, 10.0));
        }
        let mut columns = columns(&bounds);
        let index = VisibilityIndex::default();
        let mut hits = Vec::new();
        let at_origin = WorldBounds::new(WorldPoint::new(5.0, 5.0), WorldSize::new(0.0, 0.0));

        index.query(&columns, at_origin, &mut hits);
        assert_eq!(hits, vec![0], "never-built grid still answers correctly");

        columns.set_position(0, WorldPoint::new(9_000.0, 9_000.0));
        index.query(&columns, at_origin, &mut hits);
        assert!(hits.is_empty(), "a moved node is not reported at its old place");
    }

    #[test]
    fn the_governor_sheds_detail_when_frames_run_long_and_restores_it_when_they_do_not() {
        let mut governor = DetailGovernor::default();
        assert_eq!(governor.detail(), 1.0);
        assert_eq!(governor.stride(), 1);

        for _ in 0..60 {
            governor.observe(40.0, 16.7);
        }
        let shed = governor.detail();
        assert!(shed < 0.5, "detail should fall well below full: {shed}");
        assert!(governor.stride() > 1);

        for _ in 0..400 {
            governor.observe(8.0, 16.7);
        }
        assert_eq!(governor.detail(), 1.0, "detail returns once frames are cheap");
    }

    #[test]
    fn the_governor_never_sheds_the_whole_graph_or_reacts_to_nonsense() {
        let mut governor = DetailGovernor::default();
        for _ in 0..10_000 {
            governor.observe(500.0, 16.7);
        }
        assert!(governor.detail() >= 0.05, "a floor keeps the drawing legible");

        let steady = governor.detail();
        governor.observe(f32::NAN, 16.7);
        governor.observe(-1.0, 16.7);
        governor.observe(10.0, 0.0);
        assert_eq!(governor.detail(), steady);
    }
    #[test]
    fn moving_one_node_dirties_only_it_and_adjacent_edges() {
        let edges = vec![
            Edge::new(1u64, 2u64).with_id(10u64),
            Edge::new(2u64, 3u64).with_id(11u64),
            Edge::new(4u64, 5u64).with_id(12u64),
        ];
        let mut tracker = DirtyTracker::default();
        tracker.rebuild(&edges);
        tracker.take();
        tracker.mark_node(NodeId(2));
        let dirty = tracker.take();
        assert_eq!(dirty.nodes, HashSet::from([NodeId(2)]));
        assert_eq!(dirty.edges, HashSet::from([EdgeId(10), EdgeId(11)]));
    }
    #[test]
    fn geometry_cache_reuses_matching_revision_stamp() {
        let mut cache = GeometryCache::default();
        let mut computes = 0;
        assert_eq!(
            cache.get_or_insert_with(EdgeId(1), (1, 2, 3), || {
                computes += 1;
                7
            }),
            7
        );
        assert_eq!(
            cache.get_or_insert_with(EdgeId(1), (1, 2, 3), || {
                computes += 1;
                8
            }),
            7
        );
        assert_eq!(computes, 1);
        assert_eq!(
            cache.get_or_insert_with(EdgeId(1), (1, 3, 3), || {
                computes += 1;
                9
            }),
            9
        );
        assert_eq!(computes, 2);
    }
}

/// Closed-loop level-of-detail control.
///
/// A renderer cannot know in advance what a frame will cost. The same graph is
/// cheap once a force-directed layout settles and expensive while it is still
/// spreading, because the edges are then long enough to stripe the whole
/// screen. Choosing a fixed edge budget therefore means choosing between a
/// frame rate that collapses during the expensive phase and detail that is
/// needlessly poor during the cheap one.
///
/// So the budget is measured rather than assumed: time the frames that
/// actually happened, and if they are missing the deadline, draw a smaller
/// sample of the edges next frame; if they are comfortably inside it, put
/// detail back. Detail falls faster than it rises, which keeps the control
/// loop from oscillating visibly around the point where the frame budget runs
/// out.
#[derive(Clone, Debug)]
pub struct DetailGovernor {
    average_ms: f32,
    /// Fraction of edges drawn, in `MIN_DETAIL..=1.0`.
    detail: f32,
}

impl Default for DetailGovernor {
    fn default() -> Self {
        Self {
            average_ms: 0.0,
            detail: 1.0,
        }
    }
}

impl DetailGovernor {
    /// Never sample below one edge in twenty: past that the drawing stops
    /// describing the graph, and a frame rate bought that way is not worth it.
    const MIN_DETAIL: f32 = 0.05;
    /// Weight of the newest frame in the running average. Low enough that one
    /// slow frame does not visibly drop detail, high enough to react within a
    /// few frames of a sustained change.
    const SMOOTHING: f32 = 0.1;

    /// Folds one frame's duration into the average and returns the detail
    /// fraction the next frame should draw at.
    ///
    /// `target_ms` is the frame deadline. The deadband around it matters: with
    /// vsync a healthy frame lands exactly on the refresh period, so detail is
    /// only reduced once frames are clearly past the deadline rather than
    /// merely at it.
    pub fn observe(&mut self, frame_ms: f32, target_ms: f32) -> f32 {
        if !frame_ms.is_finite() || frame_ms <= 0.0 || !target_ms.is_finite() || target_ms <= 0.0 {
            return self.detail;
        }
        self.average_ms = if self.average_ms == 0.0 {
            frame_ms
        } else {
            self.average_ms + (frame_ms - self.average_ms) * Self::SMOOTHING
        };
        if self.average_ms > target_ms * 1.2 {
            self.detail = (self.detail * 0.85).max(Self::MIN_DETAIL);
        } else if self.average_ms < target_ms * 1.05 {
            self.detail = (self.detail * 1.04).min(1.0);
        }
        self.detail
    }

    pub fn detail(&self) -> f32 {
        self.detail
    }

    pub fn average_ms(&self) -> f32 {
        self.average_ms
    }

    /// Draw every nth edge to realize the current detail fraction.
    pub fn stride(&self) -> usize {
        ((1.0 / self.detail.max(Self::MIN_DETAIL)).round() as usize).max(1)
    }
}
