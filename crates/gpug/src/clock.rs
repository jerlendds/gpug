//! Monotonic clock shared by native and browser graph paths.
#[cfg(not(target_family = "wasm"))]
pub(crate) use std::time::Instant;
#[cfg(target_family = "wasm")]
pub(crate) use web_time::Instant;
