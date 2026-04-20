pub mod client;
pub mod error;
pub mod model;

pub use client::{TranscriptionClient, TranscriptionResult};
pub use error::TranscriptionError;
pub use model::{ModelInfo, ModelManager, ModelSize, ParakeetVariant, SortformerVariant};

/// Backward-compatible alias for the renamed [`TranscriptionClient`].
/// Existing call sites in the Tauri layer migrate in the engine-aware
/// command commit; the alias goes away then.
pub type WhisperClient = TranscriptionClient;
