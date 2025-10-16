use gpui::*;

use crate::node::GpugNode;

// Simple xorshift-based PRNG to avoid external dependencies
pub fn rng_next(seed: &mut u64) -> u64 {
    // Xorshift64*
    let mut x = *seed;
    if x == 0 {
        x = 0x9E3779B97F4A7C15; // avoid zero state
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    x
}

pub fn rand_f32(seed: &mut u64) -> f32 {
    // Convert upper bits to [0,1)
    let r = rng_next(seed);
    let v = (r >> 11) as u32; // 53 bits -> 42 bits -> fit in f32 mantissa
    (v as f32) / (u32::MAX as f32)
}

// Generate n nodes with random positions within a region
pub fn generate_nodes(n: usize) -> Vec<GpugNode> {
    let mut seed: u64 = 0xCAFEBABEDEADBEEF;
    let mut nodes: Vec<GpugNode> = Vec::with_capacity(n);

    // Scatter in a reasonable viewport box
    let left = 50.0f32;
    let top = 50.0f32;
    let width = 1200.0f32;
    let height = 800.0f32;

    for i in 0..n {
        let rx = rand_f32(&mut seed);
        let ry = rand_f32(&mut seed);
        let x = px(left + rx * width);
        let y = px(top + ry * height);
        nodes.push(GpugNode {
            id: (i as u64) + 1,
            x,
            y,
            drag_offset: None,
            zoom: 1.0,
            pan: point(px(0.0), px(0.0)),
            selected: false,
        });
    }
    nodes
}
