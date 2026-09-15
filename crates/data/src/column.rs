//! Native Vortex buffers shared by analytics and the mutable rendering workspace.
//! A slice is resolved before each scan; no Vortex scalar dispatch or decompression
//! is performed per graph item. The optional benchmark backend changes storage
//! only, preserving the algorithms and work performed by each scan.
use std::ops::{Deref, DerefMut};
use vortex::buffer::{Buffer, BufferMut};

/// True only in a benchmark-enabled build explicitly selecting the Vec baseline.
/// Selection is read once per process, before any graph buffers are allocated.
pub fn uses_vec_backend() -> bool {
    #[cfg(feature = "benchmark-backends")]
    {
        static BASELINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *BASELINE.get_or_init(|| std::env::var("GPUG_BENCH_BACKEND").is_ok_and(|v| v == "vec"))
    }
    #[cfg(not(feature = "benchmark-backends"))]
    false
}

#[derive(Clone, Debug)]
enum MutableStorage<T> {
    Vortex(BufferMut<T>),
    #[cfg(feature = "benchmark-backends")]
    Vec(Vec<T>),
}

/// Mutable native column for runtime geometry and adjacency construction.
/// Frozen columns share Vortex allocations without copying. Live editor columns
/// remain mutable so dragging never decompresses or rebuilds a graph snapshot.
#[derive(Clone, Debug)]
pub struct Column<T>(MutableStorage<T>);

impl<T> Default for Column<T> {
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl<T> Column<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        #[cfg(feature = "benchmark-backends")]
        if uses_vec_backend() {
            return Self(MutableStorage::Vec(Vec::with_capacity(capacity)));
        }
        Self(MutableStorage::Vortex(BufferMut::with_capacity(capacity)))
    }
    pub fn clear(&mut self) {
        match &mut self.0 {
            MutableStorage::Vortex(v) => v.clear(),
            #[cfg(feature = "benchmark-backends")]
            MutableStorage::Vec(v) => v.clear(),
        }
    }
    pub fn push(&mut self, value: T) {
        match &mut self.0 {
            MutableStorage::Vortex(v) => v.push(value),
            #[cfg(feature = "benchmark-backends")]
            MutableStorage::Vec(v) => v.push(value),
        }
    }
    pub fn resize(&mut self, len: usize, value: T)
    where
        T: Copy,
    {
        match &mut self.0 {
            MutableStorage::Vortex(v) => {
                v.truncate(len);
                v.push_n(value, len.saturating_sub(v.len()));
            }
            #[cfg(feature = "benchmark-backends")]
            MutableStorage::Vec(v) => v.resize(len, value),
        }
    }
    pub fn freeze(self) -> FrozenColumn<T> {
        FrozenColumn(match self.0 {
            MutableStorage::Vortex(v) => FrozenStorage::Vortex(v.freeze()),
            #[cfg(feature = "benchmark-backends")]
            MutableStorage::Vec(v) => FrozenStorage::Vec(v.into()),
        })
    }
}
impl<T> Deref for Column<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match &self.0 {
            MutableStorage::Vortex(v) => v.as_slice(),
            #[cfg(feature = "benchmark-backends")]
            MutableStorage::Vec(v) => v,
        }
    }
}
impl<T> DerefMut for Column<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match &mut self.0 {
            MutableStorage::Vortex(v) => v.as_mut_slice(),
            #[cfg(feature = "benchmark-backends")]
            MutableStorage::Vec(v) => v,
        }
    }
}
impl<T: PartialEq> PartialEq<Vec<T>> for Column<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        &**self == other
    }
}

#[derive(Clone, Debug)]
enum FrozenStorage<T> {
    Vortex(Buffer<T>),
    #[cfg(feature = "benchmark-backends")]
    Vec(std::sync::Arc<[T]>),
}

/// Immutable native column. Cloning shares its allocation; slices are suitable
/// for adjacency scans, layout input and renderer uploads without materialization.
#[derive(Clone, Debug)]
pub struct FrozenColumn<T>(FrozenStorage<T>);
impl<T> Deref for FrozenColumn<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match &self.0 {
            FrozenStorage::Vortex(v) => v.as_slice(),
            #[cfg(feature = "benchmark-backends")]
            FrozenStorage::Vec(v) => v,
        }
    }
}
impl<T: Send + Sync + 'static> From<Vec<T>> for FrozenColumn<T> {
    fn from(value: Vec<T>) -> Self {
        #[cfg(feature = "benchmark-backends")]
        if uses_vec_backend() {
            return Self(FrozenStorage::Vec(value.into()));
        }
        Self(FrozenStorage::Vortex(value.into()))
    }
}
impl<T> From<Buffer<T>> for FrozenColumn<T> {
    fn from(value: Buffer<T>) -> Self {
        Self(FrozenStorage::Vortex(value))
    }
}
impl<T> Default for FrozenColumn<T> {
    fn default() -> Self {
        Column::default().freeze()
    }
}
