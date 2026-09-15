use super::{Bounds, Children, SpatialIndex};
use crate::{NodeId, WorldRect};
use std::{fmt, ops::Range, sync::Arc};

/// Node representation controls and deterministic work budgets. At least one item
/// and branch visit must be permitted to retain a coarse fallback. max_nodes may
/// be zero: the plan then uses precomputed branch bounds/counts only.
#[derive(Clone, Copy, Debug)]
pub struct LodOptions {
    pub zoom: f64,
    pub max_zoom: f64,
    pub full_detail_zoom_ratio: f64,
    pub aggregate_below_pixels: f64,
    pub max_items: usize,
    pub max_branches: usize,
    pub max_nodes: usize,
}
impl Default for LodOptions {
    fn default() -> Self {
        Self {
            zoom: 1.,
            max_zoom: 60.,
            full_detail_zoom_ratio: 0.9,
            aggregate_below_pixels: 24.,
            max_items: 512,
            max_branches: 8192,
            max_nodes: 8192,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidLodOptions;
impl fmt::Display for InvalidLodOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LOD requires a valid viewport, positive finite zoom scales, ratio in (0,1], nonnegative finite pixel threshold, and nonzero item/branch budgets")
    }
}
impl std::error::Error for InvalidLodOptions {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodKind {
    Node(NodeId),
    Aggregate,
}

/// An aggregate represents an exact source subset, which may include off-screen
/// nodes. Its count is NOT an exact visible-node count. Aggregate bounds are the
/// subtree's bounding box clipped to the viewport; individual bounds are unchanged.
/// Use the plan's members() iterator to resolve provenance, never synthetic node IDs.
#[derive(Clone, Debug)]
pub struct LodItem {
    pub kind: LodKind,
    pub bounds: WorldRect,
    pub represented_nodes: usize,
    pub(super) members: Range<usize>,
    budget_limited: bool,
}

/// Worker-publishable plan. Only bounded item descriptors are copied; the index
/// and immutable graph stay shared. Host request/viewport stamps are still needed
/// to reject a plan prepared for an obsolete interaction.
#[derive(Debug)]
pub struct OwnedLodPlan {
    pub(super) index: Arc<SpatialIndex>,
    pub(super) items: Vec<LodItem>,
    pub visited_branches: usize,
    pub examined_nodes: usize,
    pub budget_aggregates: usize,
}
impl OwnedLodPlan {
    pub fn index(&self) -> &Arc<SpatialIndex> {
        &self.index
    }
    pub fn items(&self) -> &[LodItem] {
        &self.items
    }
    pub fn members(&self, item: usize) -> Option<impl ExactSizeIterator<Item = NodeId> + '_> {
        let range = self.items.get(item)?.members.clone();
        Some(
            self.index.indices[range]
                .iter()
                .map(|dense| self.index.graph.nodes()[*dense as usize].id),
        )
    }
}

/// A disjoint representation cover of every non-hidden node intersecting the
/// viewport. Coarse aggregates may conservatively cover extra source nodes.
/// Membership resolution remains bound to the indexed snapshot. This is a node
/// plan only: it does not omit or aggregate edges and cannot serve as an edge mask.
#[derive(Debug)]
pub struct LodPlan<'a> {
    index: &'a SpatialIndex,
    items: &'a [LodItem],
    pub visited_branches: usize,
    pub examined_nodes: usize,
    pub budget_aggregates: usize,
}
impl LodPlan<'_> {
    pub fn items(&self) -> &[LodItem] {
        self.items
    }
    /// Exact source identities for a plan item, lazily resolved without copying.
    /// Resolving a coarse item's full membership can be O(V); do this off paint.
    pub fn members(&self, item: usize) -> Option<impl ExactSizeIterator<Item = NodeId> + '_> {
        let range = self.items.get(item)?.members.clone();
        Some(
            self.index.indices[range]
                .iter()
                .map(|dense| self.index.graph.nodes()[*dense as usize].id),
        )
    }
}

#[derive(Debug, Default)]
pub struct LodWorkspace {
    stack: Vec<usize>,
    items: Vec<LodItem>,
}
impl LodWorkspace {
    pub fn new() -> Self {
        Self::default()
    }
    /// Prepare a view with one visible source node prioritized as detailed content.
    /// Reserves two item slots and one node examination before normal planning,
    /// then extracts the focused member from its aggregate without scanning its
    /// membership. Residual ranges retain conservative bounds and exact counts.
    /// The focused item is last, so bridge consumers paint/hit-test it above the
    /// residual aggregates rather than letting them cover its custom content.
    /// Requires max_items >= 3 and max_nodes >= 1; smaller budgets retain the
    /// ordinary conservative plan. Unknown, hidden or offscreen focus is ignored.
    /// This priority applies at every zoom; hosts may request it only near max zoom.
    pub fn prepare_focused(
        &mut self,
        index: Arc<SpatialIndex>,
        viewport: WorldRect,
        options: LodOptions,
        focus: Option<NodeId>,
    ) -> Result<OwnedLodPlan, InvalidLodOptions> {
        let focused = focus.and_then(|id| {
            let dense = index.graph.node_index(id)? as usize;
            let node = &index.graph.nodes()[dense];
            (!node.hidden && Bounds::from(node.bounds).intersects(Bounds::from(viewport)))
                .then_some((id, node.bounds, index.member_positions[dense] as usize))
        });
        let Some((id, bounds, member)) =
            focused.filter(|_| options.max_items >= 3 && options.max_nodes >= 1)
        else {
            return self.prepare(index, viewport, options);
        };
        let mut reduced = options;
        reduced.max_items -= 2;
        reduced.max_nodes -= 1;
        let mut plan = self.prepare(index, viewport, reduced)?;
        plan.examined_nodes += 1;
        let Some(slot) = plan
            .items
            .iter()
            .position(|item| item.members.contains(&member))
        else {
            return Ok(plan);
        };
        if matches!(plan.items[slot].kind, LodKind::Node(_)) {
            let focused = plan.items.remove(slot);
            plan.items.push(focused);
            return Ok(plan);
        }
        let original = &plan.items[slot];
        let residual = |members: Range<usize>| {
            (!members.is_empty()).then_some(LodItem {
                kind: LodKind::Aggregate,
                bounds: original.bounds,
                represented_nodes: members.len(),
                members,
                budget_limited: original.budget_limited,
            })
        };
        let before = residual(original.members.start..member);
        let after = residual(member + 1..original.members.end);
        if original.budget_limited {
            plan.budget_aggregates = plan.budget_aggregates - 1
                + usize::from(before.is_some())
                + usize::from(after.is_some());
        }
        plan.items
            .splice(slot..slot + 1, [before, after].into_iter().flatten());
        plan.items.push(LodItem {
            kind: LodKind::Node(id),
            bounds,
            represented_nodes: 1,
            members: member..member + 1,
            budget_limited: false,
        });
        Ok(plan)
    }
    /// Prepare bounded owned descriptors for publication while retaining scratch
    /// capacity. Does not copy the source graph or enumerate aggregate members.
    pub fn prepare(
        &mut self,
        index: Arc<SpatialIndex>,
        viewport: WorldRect,
        options: LodOptions,
    ) -> Result<OwnedLodPlan, InvalidLodOptions> {
        let plan = self.plan(&index, viewport, options)?;
        let items = plan.items.to_vec();
        let (visited_branches, examined_nodes, budget_aggregates) = (
            plan.visited_branches,
            plan.examined_nodes,
            plan.budget_aggregates,
        );
        Ok(OwnedLodPlan {
            index,
            items,
            visited_branches,
            examined_nodes,
            budget_aggregates,
        })
    }
    /// Refines a conservative root cover while reserving enough output/work slots
    /// for every queued branch. Budget exhaustion leaves aggregates, never holes.
    /// Near maximum zoom the screen-size aggregation threshold is ignored, but
    /// hard work/output budgets remain. The planner is stateless across views;
    /// hysteresis is not implemented. Use prepare_focused for source priority.
    pub fn plan<'a>(
        &'a mut self,
        index: &'a SpatialIndex,
        viewport: WorldRect,
        options: LodOptions,
    ) -> Result<LodPlan<'a>, InvalidLodOptions> {
        if !viewport.is_valid()
            || !options.zoom.is_finite()
            || options.zoom <= 0.
            || !options.max_zoom.is_finite()
            || options.max_zoom <= 0.
            || !options.full_detail_zoom_ratio.is_finite()
            || options.full_detail_zoom_ratio <= 0.
            || options.full_detail_zoom_ratio > 1.
            || !options.aggregate_below_pixels.is_finite()
            || options.aggregate_below_pixels < 0.
            || options.max_items == 0
            || options.max_branches == 0
        {
            return Err(InvalidLodOptions);
        }
        self.stack.clear();
        self.items.clear();
        if !index.branches.is_empty() {
            self.stack.push(0);
        }
        let viewport = Bounds::from(viewport);
        let full_detail = options.zoom >= options.max_zoom * options.full_detail_zoom_ratio;
        let mut visited_branches = 0;
        let mut examined_nodes = 0;
        let mut budget_aggregates = 0;
        while let Some(slot) = self.stack.pop() {
            visited_branches += 1;
            let branch = &index.branches[slot];
            if !branch.bounds.intersects(viewport) {
                continue;
            }
            let projected_size = (branch.bounds.max_x - branch.bounds.min_x)
                .max(branch.bounds.max_y - branch.bounds.min_y)
                * options.zoom;
            let small = !full_detail && projected_size <= options.aggregate_below_pixels;
            let available_items = options.max_items - self.items.len() - self.stack.len();
            let mut budget_limited = false;
            if !small {
                match branch.children {
                    Children::Split { left, right } => {
                        // Each queued branch needs one future visit and output
                        // reservation. Subtraction avoids overflowing huge budgets.
                        if available_items >= 2
                            && options.max_branches - visited_branches - self.stack.len() >= 2
                        {
                            self.stack.push(right);
                            self.stack.push(left);
                            continue;
                        }
                        budget_limited = true;
                    }
                    Children::Leaf { start, end } => {
                        let count = end - start;
                        if available_items >= count && options.max_nodes - examined_nodes >= count {
                            for member in start..end {
                                examined_nodes += 1;
                                let node = &index.graph.nodes()[index.indices[member] as usize];
                                if Bounds::from(node.bounds).intersects(viewport) {
                                    self.items.push(LodItem {
                                        kind: LodKind::Node(node.id),
                                        bounds: node.bounds,
                                        represented_nodes: 1,
                                        members: member..member + 1,
                                        budget_limited: false,
                                    });
                                }
                            }
                            continue;
                        }
                        budget_limited = true;
                    }
                }
            }
            budget_aggregates += usize::from(budget_limited);
            self.items.push(LodItem {
                kind: LodKind::Aggregate,
                bounds: clipped(branch.bounds, viewport),
                represented_nodes: branch.end - branch.start,
                members: branch.start..branch.end,
                budget_limited,
            });
        }
        Ok(LodPlan {
            index,
            items: &self.items,
            visited_branches,
            examined_nodes,
            budget_aggregates,
        })
    }
}

fn clipped(bounds: Bounds, viewport: Bounds) -> WorldRect {
    let x = bounds.min_x.max(viewport.min_x);
    let y = bounds.min_y.max(viewport.min_y);
    WorldRect {
        x,
        y,
        width: bounds.max_x.min(viewport.max_x) - x,
        height: bounds.max_y.min(viewport.max_y) - y,
    }
}
