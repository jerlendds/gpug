//! Framework-neutral graph snapshots and analytics for GPUG.
//!
//! Stable external IDs are separate from dense snapshot-local indices. Snapshot
//! construction validates the complete graph before publishing adjacency indexes.
//! Share snapshots with `Arc`; run construction and analytics off the UI thread.
#![forbid(unsafe_code)]

mod analytics;
mod model;
mod path;
pub mod query;
pub mod spatial;
pub mod worker;

#[cfg(feature = "gpug")]
pub mod gpug_bridge;

#[cfg(feature = "duckdb")]
pub mod duckdb_store;

#[cfg(feature = "vortex")]
pub mod vortex_io;

pub use analytics::{Direction, Neighborhood, SearchBudget, SearchWorkspace};
pub use model::{Edge, EdgeId, GraphError, GraphSnapshot, Mutation, Node, NodeId, WorldRect};
pub use path::{PathResult, PathStatus, PathStop, PathWorkspace};
