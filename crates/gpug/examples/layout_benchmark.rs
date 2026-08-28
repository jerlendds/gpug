use gpug::{Edge, ForceAtlas2, Layout, SmallWorld, WorldPoint};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

#[inline(never)]
fn read_position(positions: &[(f32, f32)], index: usize) -> (f32, f32) {
    black_box(positions[index])
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn baseline_step(xs: &mut [f32], ys: &mut [f32], edges: &[Edge]) {
    let n = xs.len();
    let mut fx = vec![0.0f32; n];
    let mut fy = vec![0.0f32; n];
    let mut bins: HashMap<(i32, i32), Vec<usize>> = HashMap::with_capacity(n * 2);
    for i in 0..n {
        bins.entry((
            (xs[i] / 100.0).floor() as i32,
            (ys[i] / 100.0).floor() as i32,
        ))
        .or_default()
        .push(i);
    }
    for i in 0..n {
        let gx = (xs[i] / 100.0).floor() as i32;
        let gy = (ys[i] / 100.0).floor() as i32;
        for dxg in -1..=1 {
            for dyg in -1..=1 {
                if let Some(indices) = bins.get(&(gx + dxg, gy + dyg)) {
                    for &j in indices {
                        if j <= i {
                            continue;
                        }
                        let dx = xs[j] - xs[i];
                        let dy = ys[j] - ys[i];
                        let inv = 1.0 / (dx * dx + dy * dy + 0.01);
                        let x_force = 120.0 * dx * inv;
                        let y_force = 120.0 * dy * inv;
                        fx[i] -= x_force;
                        fy[i] -= y_force;
                        fx[j] += x_force;
                        fy[j] += y_force;
                    }
                }
            }
        }
    }
    for edge in edges {
        let (i, j) = (edge.source.index(), edge.target.index());
        if i >= n || j >= n {
            continue;
        }
        let dx = xs[j] - xs[i];
        let dy = ys[j] - ys[i];
        fx[i] += 0.03 * dx;
        fy[i] += 0.03 * dy;
        fx[j] -= 0.03 * dx;
        fy[j] -= 0.03 * dy;
    }
    for i in 0..n {
        let mut dx = (fx[i] + 0.006 * (800.0 - xs[i])) * 0.425;
        let mut dy = (fy[i] + 0.006 * (200.0 - ys[i])) * 0.425;
        let d2 = dx * dx + dy * dy;
        if d2 > 25.0 {
            let scale = 5.0 / d2.sqrt();
            dx *= scale;
            dy *= scale;
        }
        xs[i] += dx;
        ys[i] += dy;
    }
}

fn main() {
    let nodes = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let frames = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let probability = std::env::args()
        .nth(3)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.00001);
    let samples = std::env::args()
        .nth(4)
        .and_then(|v| v.parse().ok())
        .unwrap_or(3usize)
        .max(1);
    let data = SmallWorld::new(nodes)
        .local_neighbors(3)
        .shortcut_probability(probability)
        .seed(42)
        .generate();
    let layout_edges = data.compile_edges().unwrap();
    let edges = data.edges;
    let original_xs: Vec<_> = data.nodes.iter().map(|node| node.position.x).collect();
    let original_ys: Vec<_> = data.nodes.iter().map(|node| node.position.y).collect();

    let run = |optimized: bool| {
        let started = Instant::now();
        if optimized {
            let mut positions: Vec<_> = original_xs
                .iter()
                .copied()
                .zip(original_ys.iter().copied())
                .map(|(x, y)| WorldPoint::new(x, y))
                .collect();
            let mut layout = ForceAtlas2::default();
            for _ in 0..frames {
                let _ = layout.step(&mut positions, &layout_edges);
            }
            black_box(positions);
        } else {
            let (mut xs, mut ys) = (original_xs.clone(), original_ys.clone());
            for _ in 0..frames {
                baseline_step(&mut xs, &mut ys, &edges);
            }
            black_box((&xs, &ys));
        }
        started.elapsed().as_secs_f64() * 1_000.0
    };
    let mut baseline_samples = Vec::with_capacity(samples);
    let mut optimized_samples = Vec::with_capacity(samples);
    for sample in 0..samples {
        if sample % 2 == 0 {
            baseline_samples.push(run(false));
            optimized_samples.push(run(true));
        } else {
            optimized_samples.push(run(true));
            baseline_samples.push(run(false));
        }
    }
    let baseline_ms = median(baseline_samples);
    let optimized_ms = median(optimized_samples);
    let render_positions: Vec<_> = original_xs
        .iter()
        .copied()
        .zip(original_ys.iter().copied())
        .collect();
    let render_run = |optimized: bool| {
        let started = Instant::now();
        for _ in 0..frames {
            if optimized {
                let cached: Vec<_> = (0..nodes)
                    .map(|i| read_position(&render_positions, i))
                    .collect();
                let stride = edges.len().div_ceil(100_000).max(1);
                for edge in edges.iter().step_by(stride) {
                    let (x1, y1) = cached[edge.source.index()];
                    let (x2, y2) = cached[edge.target.index()];
                    let dx = x2 - x1;
                    let dy = y2 - y1;
                    let inverse_length = (dx * dx + dy * dy + 0.0001).sqrt().recip();
                    let nx = -dy * inverse_length * 0.5;
                    let ny = dx * inverse_length * 0.5;
                    black_box((
                        (x1 + nx, y1 + ny),
                        (x1 - nx, y1 - ny),
                        (x2 + nx, y2 + ny),
                        (x2 - nx, y2 - ny),
                    ));
                }
            } else {
                for edge in &edges {
                    let (x1, y1) = read_position(&render_positions, edge.source.index());
                    let (x2, y2) = read_position(&render_positions, edge.target.index());
                    let dx = x2 - x1;
                    let dy = y2 - y1;
                    let inverse_length = (dx * dx + dy * dy + 0.0001).sqrt().recip();
                    let nx = -dy * inverse_length * 0.5;
                    let ny = dx * inverse_length * 0.5;
                    black_box((
                        (x1 + nx, y1 + ny),
                        (x1 - nx, y1 - ny),
                        (x2 + nx, y2 + ny),
                        (x2 - nx, y2 - ny),
                    ));
                }
            }
        }
        started.elapsed().as_secs_f64() * 1_000.0
    };
    let render_baseline_ms = median((0..samples).map(|_| render_run(false)).collect());
    let render_optimized_ms = median((0..samples).map(|_| render_run(true)).collect());
    println!("{{\"model\":\"small_world\",\"nodes\":{nodes},\"edges\":{},\"local_neighbors\":3,\"probability\":{probability},\"frames\":{frames},\"samples\":{samples},\"layout_baseline_ms\":{baseline_ms:.3},\"layout_current_ms\":{optimized_ms:.3},\"layout_ms_per_frame\":{:.3},\"layout_speedup\":{:.3},\"render_baseline_ms\":{render_baseline_ms:.3},\"render_optimized_ms\":{render_optimized_ms:.3},\"render_speedup\":{:.3}}}", edges.len(), optimized_ms / frames as f64, baseline_ms / optimized_ms, render_baseline_ms / render_optimized_ms);
}
