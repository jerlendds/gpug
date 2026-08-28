use crate::coordinates::WorldPoint;
use crate::node::Node;

// Simple xorshift-based PRNG to avoid external dependencies
pub(crate) fn rng_next(seed: &mut u64) -> u64 {
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

pub(crate) fn rand_f32(seed: &mut u64) -> f32 {
    // Convert upper bits to [0,1)
    let r = rng_next(seed);
    let v = (r >> 11) as u32; // 53 bits -> 42 bits -> fit in f32 mantissa
    (v as f32) / (u32::MAX as f32)
}

pub(crate) fn rand_f64(seed: &mut u64) -> f64 {
    let value = rng_next(seed) >> 11;
    value as f64 * (1.0 / ((1u64 << 53) as f64))
}

// Generate n nodes with random positions within a region
pub fn generate_nodes(n: usize) -> Vec<Node> {
    generate_nodes_with_seed(n, 0xCAFEBABEDEADBEEF)
}

pub fn generate_nodes_with_seed(n: usize, mut seed: u64) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(n);

    // Scatter in a reasonable viewport box
    let left = 50.0f32;
    let top = 50.0f32;
    let width = 1200.0f32;
    let height = 800.0f32;

    for i in 0..n {
        let rx = rand_f32(&mut seed);
        let ry = rand_f32(&mut seed);
        nodes.push(Node::new(
            i,
            WorldPoint::new(left + rx * width, top + ry * height),
        ));
    }
    nodes
}
