use rayon::prelude::*;

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
    bodies: Vec<usize>,
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
    fx: Vec<f32>,
    fy: Vec<f32>,
    old_fx: Vec<f32>,
    old_fy: Vec<f32>,
    adjacency_offsets: Vec<usize>,
    adjacency_targets: Vec<usize>,
    topology_key: (usize, usize, usize),
    tree: Vec<Quad>,
    speed: f32,
    speed_efficiency: f32,
}

impl Default for LayoutWorkspace {
    fn default() -> Self {
        Self {
            fx: Vec::new(),
            fy: Vec::new(),
            old_fx: Vec::new(),
            old_fy: Vec::new(),
            adjacency_offsets: Vec::new(),
            adjacency_targets: Vec::new(),
            topology_key: (0, 0, 0),
            tree: Vec::new(),
            speed: 1.0,
            speed_efficiency: 1.0,
        }
    }
}

impl LayoutWorkspace {
    fn rebuild_adjacency(&mut self, node_count: usize, edges: &[LayoutEdge]) {
        let pointer = edges.as_ptr() as usize;
        let key = (node_count, edges.len(), pointer);
        if self.topology_key == key && self.adjacency_offsets.len() == node_count + 1 {
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
        self.topology_key = key;
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
        let bodies: Vec<_> = (0..xs.len()).collect();
        build_quad(
            &mut self.tree,
            xs,
            ys,
            bodies,
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
                accumulate_repulsion(tree, 0, index, x, y, &mut repulsion_x, &mut repulsion_y);
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
    xs: &[f32],
    ys: &[f32],
    bodies: Vec<usize>,
    center_x: f32,
    center_y: f32,
    half_size: f32,
    depth: usize,
) -> usize {
    let index = tree.len();
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
        bodies: Vec::new(),
    });
    if bodies.len() <= 1 || depth >= MAX_TREE_DEPTH {
        tree[index].bodies = bodies;
        return index;
    }

    let mut partitions = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for body in bodies {
        let quadrant = usize::from(xs[body] >= center_x) | (usize::from(ys[body] >= center_y) << 1);
        partitions[quadrant].push(body);
    }
    let child_half = half_size * 0.5;
    for (quadrant, partition) in partitions.into_iter().enumerate() {
        if partition.is_empty() {
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
            xs,
            ys,
            partition,
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
    quad_index: usize,
    body: usize,
    x: f32,
    y: f32,
    force_x: &mut f32,
    force_y: &mut f32,
) {
    let quad = &tree[quad_index];
    if !quad.bodies.is_empty() {
        for &other in &quad.bodies {
            if other == body {
                continue;
            }
            let dx = quad.mass_x - x;
            let dy = quad.mass_y - y;
            let inverse_distance = 1.0 / (dx * dx + dy * dy + 0.01);
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
            accumulate_repulsion(tree, child, body, x, y, force_x, force_y);
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
}
