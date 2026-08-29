use rayon::prelude::*;

use crate::coordinates::WorldPoint;
use crate::data::LayoutEdge;

const NONE: usize = usize::MAX;
const BARNES_HUT_THETA: f32 = 1.2;
const MAX_TREE_DEPTH: usize = 20;

#[derive(Clone)]
struct Quad {
    center_x: f32,
    center_y: f32,
    half_size: f32,
    mass_x: f32,
    mass_y: f32,
    mass: f32,
    children: [usize; 4],
    #[cfg(test)]
    body_start: usize,
    #[cfg(test)]
    body_end: usize,
}

impl Quad {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.center_x - self.half_size
            && x <= self.center_x + self.half_size
            && y >= self.center_y - self.half_size
            && y <= self.center_y + self.half_size
    }
}

/// Force-directed layout workspace. It keeps topology and force buffers in
/// structure-of-arrays form and uses ForceAtlas2's Barnes-Hut theta default.
pub struct LayoutWorkspace {
    position_xs: Vec<f32>,
    position_ys: Vec<f32>,
    fx: Vec<f32>,
    fy: Vec<f32>,
    old_fx: Vec<f32>,
    old_fy: Vec<f32>,
    adjacency_offsets: Vec<usize>,
    adjacency_targets: Vec<usize>,
    topology_node_count: usize,
    topology_edges: Vec<LayoutEdge>,
    tree: Vec<Quad>,
    tree_indices: Vec<usize>,
    tree_scratch: Vec<usize>,
    body_leaf: Vec<usize>,
    speed: f32,
    speed_efficiency: f32,
}

impl Default for LayoutWorkspace {
    fn default() -> Self {
        Self {
            position_xs: Vec::new(),
            position_ys: Vec::new(),
            fx: Vec::new(),
            fy: Vec::new(),
            old_fx: Vec::new(),
            old_fy: Vec::new(),
            adjacency_offsets: Vec::new(),
            adjacency_targets: Vec::new(),
            topology_node_count: 0,
            topology_edges: Vec::new(),
            tree: Vec::new(),
            tree_indices: Vec::new(),
            tree_scratch: Vec::new(),
            body_leaf: Vec::new(),
            speed: 1.0,
            speed_efficiency: 1.0,
        }
    }
}

impl LayoutWorkspace {
    /// Advances an interleaved position buffer while retaining the simulation's
    /// structure-of-arrays scratch storage across frames.
    pub fn step_positions(&mut self, positions: &mut [WorldPoint], edges: &[LayoutEdge]) -> f32 {
        let mut xs = std::mem::take(&mut self.position_xs);
        let mut ys = std::mem::take(&mut self.position_ys);
        xs.clear();
        ys.clear();
        xs.extend(positions.iter().map(|position| position.x));
        ys.extend(positions.iter().map(|position| position.y));

        let energy = self.step(&mut xs, &mut ys, edges);
        for ((position, x), y) in positions.iter_mut().zip(&xs).zip(&ys) {
            *position = WorldPoint::new(*x, *y);
        }
        self.position_xs = xs;
        self.position_ys = ys;
        energy
    }

    fn rebuild_adjacency(&mut self, node_count: usize, edges: &[LayoutEdge]) {
        if self.topology_node_count == node_count
            && self.topology_edges == edges
            && self.adjacency_offsets.len() == node_count + 1
        {
            return;
        }

        self.adjacency_offsets.clear();
        self.adjacency_offsets.resize(node_count + 1, 0);
        for edge in edges {
            let source = edge.source;
            let target = edge.target;
            if source < node_count && target < node_count {
                self.adjacency_offsets[source + 1] += 1;
                self.adjacency_offsets[target + 1] += 1;
            }
        }
        for index in 1..=node_count {
            self.adjacency_offsets[index] += self.adjacency_offsets[index - 1];
        }

        self.adjacency_targets.clear();
        self.adjacency_targets
            .resize(self.adjacency_offsets[node_count], 0);
        let mut cursors = self.adjacency_offsets[..node_count].to_vec();
        for edge in edges {
            let source = edge.source;
            let target = edge.target;
            if source < node_count && target < node_count {
                self.adjacency_targets[cursors[source]] = target;
                cursors[source] += 1;
                self.adjacency_targets[cursors[target]] = source;
                cursors[target] += 1;
            }
        }
        self.topology_node_count = node_count;
        self.topology_edges.clear();
        self.topology_edges.extend_from_slice(edges);
    }

    fn build_tree(&mut self, xs: &[f32], ys: &[f32]) {
        self.tree.clear();
        if xs.is_empty() {
            return;
        }
        let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
        for (&x, &y) in xs.iter().zip(ys) {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let center_x = (min_x + max_x) * 0.5;
        let center_y = (min_y + max_y) * 0.5;
        let half_size = ((max_x - min_x).max(max_y - min_y) * 0.5).max(1.0) + 0.01;
        self.tree_indices.clear();
        self.tree_indices.extend(0..xs.len());
        self.tree_scratch.resize(xs.len(), 0);
        self.body_leaf.resize(xs.len(), NONE);
        build_quad(
            &mut self.tree,
            &mut self.tree_indices,
            &mut self.tree_scratch,
            &mut self.body_leaf,
            xs,
            ys,
            0,
            xs.len(),
            center_x,
            center_y,
            half_size,
            0,
        );
    }

    #[inline]
    pub fn step(&mut self, xs: &mut [f32], ys: &mut [f32], edges: &[LayoutEdge]) -> f32 {
        let n = xs.len().min(ys.len());
        if n == 0 {
            return 0.0;
        }
        for (x, y) in xs[..n].iter_mut().zip(&mut ys[..n]) {
            if !x.is_finite() {
                *x = 0.0;
            }
            if !y.is_finite() {
                *y = 0.0;
            }
        }
        self.rebuild_adjacency(n, edges);
        self.build_tree(&xs[..n], &ys[..n]);
        self.fx.resize(n, 0.0);
        self.fy.resize(n, 0.0);
        self.old_fx.resize(n, 0.0);
        self.old_fy.resize(n, 0.0);
        std::mem::swap(&mut self.fx, &mut self.old_fx);
        std::mem::swap(&mut self.fy, &mut self.old_fy);
        self.fx.resize(n, 0.0);
        self.fy.resize(n, 0.0);
        self.fx.fill(0.0);
        self.fy.fill(0.0);

        const REPULSION: f32 = 120.0;
        const ATTRACTION: f32 = 0.03;
        let tree = &self.tree;
        let body_leaf = &self.body_leaf;
        let offsets = &self.adjacency_offsets;
        let targets = &self.adjacency_targets;
        self.fx
            .par_iter_mut()
            .zip(self.fy.par_iter_mut())
            .enumerate()
            .for_each(|(index, (force_x, force_y))| {
                let x = xs[index];
                let y = ys[index];
                let (mut repulsion_x, mut repulsion_y) = (0.0, 0.0);
                accumulate_repulsion(
                    tree,
                    body_leaf,
                    0,
                    index,
                    x,
                    y,
                    &mut repulsion_x,
                    &mut repulsion_y,
                );
                *force_x = REPULSION * repulsion_x;
                *force_y = REPULSION * repulsion_y;

                for &neighbor in &targets[offsets[index]..offsets[index + 1]] {
                    *force_x += ATTRACTION * (xs[neighbor] - x);
                    *force_y += ATTRACTION * (ys[neighbor] - y);
                }
            });

        // ForceAtlas2 adaptive speed: swinging detects erratic movement while
        // effective traction measures useful convergence.
        let (total_swinging, total_traction) = (0..n)
            .into_par_iter()
            .map(|index| {
                let mass = (offsets[index + 1] - offsets[index] + 1) as f32;
                let delta_x = self.old_fx[index] - self.fx[index];
                let delta_y = self.old_fy[index] - self.fy[index];
                let sum_x = self.old_fx[index] + self.fx[index];
                let sum_y = self.old_fy[index] + self.fy[index];
                (
                    mass * (delta_x * delta_x + delta_y * delta_y).sqrt(),
                    mass * 0.5 * (sum_x * sum_x + sum_y * sum_y).sqrt(),
                )
            })
            .reduce(|| (0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        if total_swinging > 0.0 && total_traction > 0.0 {
            let estimated_jitter = 0.05 * (n as f32).sqrt();
            let jitter = estimated_jitter
                .sqrt()
                .max((estimated_jitter * total_traction / (n * n) as f32).min(10.0));
            if total_swinging / total_traction > 2.0 {
                self.speed_efficiency = (self.speed_efficiency * 0.5).max(0.05);
            }
            let target_speed = jitter * self.speed_efficiency * total_traction / total_swinging;
            if total_swinging > jitter * total_traction {
                self.speed_efficiency = (self.speed_efficiency * 0.7).max(0.05);
            } else if self.speed < 1_000.0 {
                self.speed_efficiency *= 1.3;
            }
            self.speed += (target_speed - self.speed).min(0.5 * self.speed);
            self.speed = self.speed.max(0.0001);
        }

        const GRAVITY: f32 = 0.006;
        const MAX_DISPLACEMENT: f32 = 5.0;
        let speed = self.speed;
        let total_displacement = xs[..n]
            .par_iter_mut()
            .zip(ys[..n].par_iter_mut())
            .zip(self.fx.par_iter().zip(self.fy.par_iter()))
            .enumerate()
            .map(|(index, ((x, y), (force_x, force_y)))| {
                let mass = (offsets[index + 1] - offsets[index] + 1) as f32;
                let swing_x = self.old_fx[index] - *force_x;
                let swing_y = self.old_fy[index] - *force_y;
                let swinging = mass * (swing_x * swing_x + swing_y * swing_y).sqrt();
                let factor = speed / (1.0 + (speed * swinging).sqrt());
                let mut dx = (*force_x + GRAVITY * (800.0 - *x)) * factor;
                let mut dy = (*force_y + GRAVITY * (200.0 - *y)) * factor;
                let displacement_squared = dx * dx + dy * dy;
                let displacement = if displacement_squared > MAX_DISPLACEMENT * MAX_DISPLACEMENT {
                    let scale = MAX_DISPLACEMENT / displacement_squared.sqrt();
                    dx *= scale;
                    dy *= scale;
                    MAX_DISPLACEMENT
                } else {
                    displacement_squared.sqrt()
                };
                *x += dx;
                *y += dy;
                displacement
            })
            .sum::<f32>();
        total_displacement / n as f32
    }
}

#[allow(clippy::too_many_arguments)]
fn build_quad(
    tree: &mut Vec<Quad>,
    indices: &mut [usize],
    scratch: &mut [usize],
    body_leaf: &mut [usize],
    xs: &[f32],
    ys: &[f32],
    body_start: usize,
    body_end: usize,
    center_x: f32,
    center_y: f32,
    half_size: f32,
    depth: usize,
) -> usize {
    let index = tree.len();
    let bodies = &indices[body_start..body_end];
    let mass = bodies.len() as f32;
    let (sum_x, sum_y) = bodies.iter().fold((0.0, 0.0), |(sum_x, sum_y), &body| {
        (sum_x + xs[body], sum_y + ys[body])
    });
    tree.push(Quad {
        center_x,
        center_y,
        half_size,
        mass_x: sum_x / mass,
        mass_y: sum_y / mass,
        mass,
        children: [NONE; 4],
        #[cfg(test)]
        body_start,
        #[cfg(test)]
        body_end,
    });
    let first = bodies[0];
    let coincident = bodies
        .iter()
        .all(|&body| xs[body] == xs[first] && ys[body] == ys[first]);
    if bodies.len() <= 1 || coincident || depth >= MAX_TREE_DEPTH {
        for &body in bodies {
            body_leaf[body] = index;
        }
        return index;
    }

    let mut counts = [0usize; 4];
    for &body in bodies {
        let quadrant = usize::from(xs[body] >= center_x) | (usize::from(ys[body] >= center_y) << 1);
        counts[quadrant] += 1;
    }
    let mut starts = [body_start; 4];
    for quadrant in 1..4 {
        starts[quadrant] = starts[quadrant - 1] + counts[quadrant - 1];
    }
    let mut cursors = starts;
    for &body in bodies {
        let quadrant = usize::from(xs[body] >= center_x) | (usize::from(ys[body] >= center_y) << 1);
        scratch[cursors[quadrant]] = body;
        cursors[quadrant] += 1;
    }
    indices[body_start..body_end].copy_from_slice(&scratch[body_start..body_end]);

    let child_half = half_size * 0.5;
    for quadrant in 0..4 {
        if counts[quadrant] == 0 {
            continue;
        }
        let child_x = center_x
            + if quadrant & 1 == 0 {
                -child_half
            } else {
                child_half
            };
        let child_y = center_y
            + if quadrant & 2 == 0 {
                -child_half
            } else {
                child_half
            };
        tree[index].children[quadrant] = build_quad(
            tree,
            indices,
            scratch,
            body_leaf,
            xs,
            ys,
            starts[quadrant],
            starts[quadrant] + counts[quadrant],
            child_x,
            child_y,
            child_half,
            depth + 1,
        );
    }
    index
}

#[allow(clippy::too_many_arguments)]
fn accumulate_repulsion(
    tree: &[Quad],
    body_leaf: &[usize],
    quad_index: usize,
    body: usize,
    x: f32,
    y: f32,
    force_x: &mut f32,
    force_y: &mut f32,
) {
    let quad = &tree[quad_index];
    if quad.children.iter().all(|&child| child == NONE) {
        let contains_body = body_leaf[body] == quad_index;
        let mass = quad.mass - f32::from(contains_body);
        if mass > 0.0 {
            let mass_x = (quad.mass_x * quad.mass - if contains_body { x } else { 0.0 }) / mass;
            let mass_y = (quad.mass_y * quad.mass - if contains_body { y } else { 0.0 }) / mass;
            let dx = mass_x - x;
            let dy = mass_y - y;
            let inverse_distance = mass / (dx * dx + dy * dy + 0.01);
            *force_x -= dx * inverse_distance;
            *force_y -= dy * inverse_distance;
        }
        return;
    }

    let dx = quad.mass_x - x;
    let dy = quad.mass_y - y;
    let distance_squared = dx * dx + dy * dy + 0.01;
    let width = quad.half_size * 2.0;
    if !quad.contains(x, y)
        && width * width < BARNES_HUT_THETA * BARNES_HUT_THETA * distance_squared
    {
        let inverse_distance = quad.mass / distance_squared;
        *force_x -= dx * inverse_distance;
        *force_y -= dy * inverse_distance;
        return;
    }
    for &child in &quad.children {
        if child != NONE {
            accumulate_repulsion(tree, body_leaf, child, body, x, y, force_x, force_y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barnes_hut_step_stays_finite() {
        let mut xs = vec![0.0, 10.0, 20.0, 30.0];
        let mut ys = vec![0.0, 5.0, 10.0, 15.0];
        let edges = vec![
            LayoutEdge {
                source: 0,
                target: 1,
            },
            LayoutEdge {
                source: 2,
                target: 3,
            },
        ];
        let _ = LayoutWorkspace::default().step(&mut xs, &mut ys, &edges);
        assert!(xs.iter().chain(&ys).all(|value| value.is_finite()));
    }

    #[test]
    fn barnes_hut_sanitizes_non_finite_coordinates() {
        let mut xs = vec![f32::NAN, f32::INFINITY];
        let mut ys = vec![f32::NEG_INFINITY, 1.0];
        let energy = LayoutWorkspace::default().step(&mut xs, &mut ys, &[]);
        assert!(energy.is_finite());
        assert!(xs.iter().chain(&ys).all(|value| value.is_finite()));
    }

    #[test]
    fn adjacency_rebuilds_when_edges_change_in_place() {
        let mut workspace = LayoutWorkspace::default();
        let mut edges = vec![LayoutEdge {
            source: 0,
            target: 1,
        }];
        workspace.rebuild_adjacency(3, &edges);
        assert_eq!(workspace.adjacency_offsets, vec![0, 1, 2, 2]);
        assert_eq!(workspace.adjacency_targets, vec![1, 0]);

        let pointer = edges.as_ptr();
        edges[0].target = 2;
        assert_eq!(edges.as_ptr(), pointer);
        workspace.rebuild_adjacency(3, &edges);

        assert_eq!(workspace.adjacency_offsets, vec![0, 1, 1, 2]);
        assert_eq!(workspace.adjacency_targets, vec![2, 0]);
    }

    #[test]
    fn coincident_bodies_collapse_into_one_aggregate_leaf() {
        let xs = vec![42.0; 10_000];
        let ys = vec![-7.0; 10_000];
        let mut workspace = LayoutWorkspace::default();

        workspace.build_tree(&xs, &ys);

        assert_eq!(workspace.tree.len(), 1);
        assert_eq!(workspace.tree[0].body_start, 0);
        assert_eq!(workspace.tree[0].body_end, xs.len());
        assert!(workspace.body_leaf.iter().all(|&leaf| leaf == 0));

        let (mut force_x, mut force_y) = (0.0, 0.0);
        accumulate_repulsion(
            &workspace.tree,
            &workspace.body_leaf,
            0,
            0,
            xs[0],
            ys[0],
            &mut force_x,
            &mut force_y,
        );
        assert_eq!((force_x, force_y), (0.0, 0.0));
    }

    #[test]
    fn tree_storage_is_reused_between_builds() {
        let xs: Vec<_> = (0..128).map(|index| index as f32).collect();
        let ys: Vec<_> = (0..128).map(|index| (index % 11) as f32).collect();
        let mut workspace = LayoutWorkspace::default();
        workspace.build_tree(&xs, &ys);
        let capacities = (
            workspace.tree.capacity(),
            workspace.tree_indices.capacity(),
            workspace.tree_scratch.capacity(),
            workspace.body_leaf.capacity(),
        );

        workspace.build_tree(&xs, &ys);

        assert_eq!(workspace.tree_indices.len(), xs.len());
        assert_eq!(workspace.body_leaf.len(), xs.len());
        assert_eq!(
            capacities,
            (
                workspace.tree.capacity(),
                workspace.tree_indices.capacity(),
                workspace.tree_scratch.capacity(),
                workspace.body_leaf.capacity(),
            )
        );
    }
}
