//! Client crate for `yapstack-embedding-sidecar`.
//!
//! Spawns and supervises the embedding sidecar process; exposes
//! `embed` / `embed_query` / `embed_batch` to the main Tauri app.
//! Concurrency is handled by per-id oneshot waiters in the IPC layer —
//! callers can issue multiple in-flight requests without an external
//! queue.

pub mod client;
pub mod error;
pub mod supervisor;

pub use client::{EmbeddingClient, ModelInfo};
pub use error::EmbeddingError;
pub use supervisor::EmbeddingSupervisor;
