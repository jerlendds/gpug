#[derive(Clone)]
pub struct GpugEdge {
    pub source: usize,
    pub target: usize,
}

impl GpugEdge {
    pub fn new(source: usize, target: usize) -> Self {
        Self { source, target }
    }
}
