use blade_graphics as gpu;
use std::mem;

struct ReusableBuffer {
    raw: gpu::Buffer,
    size: u64,
}

/// Configuration of the Blade belt.
pub struct BufferBeltDescriptor {
    /// Kind of memory to allocate from.
    pub memory: gpu::Memory,
    pub min_chunk_size: u64,
    pub alignment: u64,
}

/// A belt of reusable buffer space.
/// Could be useful for temporary data, such as texture staging areas.
pub struct BufferBelt {
    desc: BufferBeltDescriptor,
    buffers: Vec<(ReusableBuffer, gpu::SyncPoint)>,
    active: Vec<(ReusableBuffer, u64)>,
}

impl BufferBelt {
    /// Create a new belt.
    pub fn new(desc: BufferBeltDescriptor) -> Self {
        assert_ne!(desc.alignment, 0);
        Self {
            desc,
            buffers: Vec::new(),
            active: Vec::new(),
        }
    }

    /// Destroy this belt.
    pub fn destroy(&mut self, gpu: &gpu::Context) {
        for (buffer, _) in self.buffers.drain(..) {
            gpu.destroy_buffer(buffer.raw);
        }
        for (buffer, _) in self.active.drain(..) {
            gpu.destroy_buffer(buffer.raw);
        }
    }

    /// Allocate a region of `size` bytes.
    #[profiling::function]
    pub fn alloc(&mut self, size: u64, gpu: &gpu::Context) -> gpu::BufferPiece {
        for &mut (ref rb, ref mut offset) in self.active.iter_mut() {
            let aligned = offset.next_multiple_of(self.desc.alignment);
            if aligned + size <= rb.size {
                let piece = rb.raw.at(aligned);
                *offset = aligned + size;
                return piece;
            }
        }

        let index_maybe = self
            .buffers
            .iter()
            .position(|(rb, sp)| size <= rb.size && gpu.wait_for(sp, 0));
        if let Some(index) = index_maybe {
            let (rb, _) = self.buffers.remove(index);
            let piece = rb.raw.into();
            self.active.push((rb, size));
            return piece;
        }

        let chunk_index = self.buffers.len() + self.active.len();
        // Exact-size allocations retain a new buffer for every slightly larger
        // frame: a camera sweep can accumulate gigabytes of obsolete buffers.
        // Geometric capacities bound that growth; completed smaller buffers are
        // released when no retained buffer can satisfy this request. Active or
        // in-flight GPU allocations must remain alive until their fence signals.
        let mut index = 0;
        while index < self.buffers.len() {
            if self.buffers[index].0.size < size && gpu.wait_for(&self.buffers[index].1, 0) {
                let (buffer, _) = self.buffers.swap_remove(index);
                gpu.destroy_buffer(buffer.raw);
            } else {
                index += 1;
            }
        }
        let chunk_size = chunk_capacity(size, self.desc.min_chunk_size);
        let chunk = gpu.create_buffer(gpu::BufferDesc {
            name: &format!("chunk-{}", chunk_index),
            size: chunk_size,
            memory: self.desc.memory,
        });
        let rb = ReusableBuffer {
            raw: chunk,
            size: chunk_size,
        };
        self.active.push((rb, size));
        chunk.into()
    }

    /// Allocate a region to hold the byte `data` slice contents.
    pub fn alloc_bytes(&mut self, data: &[u8], gpu: &gpu::Context) -> gpu::BufferPiece {
        assert!(!data.is_empty());
        let bp = self.alloc(data.len() as u64, gpu);
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), bp.data(), data.len());
        }
        bp
    }

    // SAFETY: T should be zeroable and ordinary data, no references, pointers, cells or other complicated data type.
    /// Allocate a region to hold the typed `data` slice contents.
    pub unsafe fn alloc_typed<T>(&mut self, data: &[T], gpu: &gpu::Context) -> gpu::BufferPiece {
        assert!(!data.is_empty());
        let type_alignment = mem::align_of::<T>() as u64;
        debug_assert_eq!(
            self.desc.alignment % type_alignment,
            0,
            "Type alignment {} is too big",
            type_alignment
        );
        let total_bytes = std::mem::size_of_val(data);
        let bp = self.alloc(total_bytes as u64, gpu);
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, bp.data(), total_bytes);
        }
        bp
    }

    /// Allocate a region to hold the POD `data` slice contents.
    pub fn alloc_pod<T: bytemuck::Pod>(
        &mut self,
        data: &[T],
        gpu: &gpu::Context,
    ) -> gpu::BufferPiece {
        unsafe { self.alloc_typed(data, gpu) }
    }

    /// Mark the actively used buffers as used by GPU with a given sync point.
    pub fn flush(&mut self, sp: &gpu::SyncPoint) {
        self.buffers
            .extend(self.active.drain(..).map(|(rb, _)| (rb, sp.clone())));
    }
}

fn chunk_capacity(size: u64, minimum: u64) -> u64 {
    let required = size.max(minimum);
    required.checked_next_power_of_two().unwrap_or(required)
}

#[cfg(test)]
mod tests {
    use super::chunk_capacity;

    #[test]
    fn growing_frames_use_geometric_buckets_instead_of_one_buffer_per_frame() {
        let capacities: std::collections::BTreeSet<_> = (4097..=65536)
            .map(|size| chunk_capacity(size, 4096))
            .collect();
        assert_eq!(
            capacities.into_iter().collect::<Vec<_>>(),
            [8192, 16384, 32768, 65536]
        );
        for size in [0, 1, 4096, 4097, 1 << 32, u64::MAX] {
            assert!(chunk_capacity(size, 4096) >= size.max(4096));
        }
    }
}
