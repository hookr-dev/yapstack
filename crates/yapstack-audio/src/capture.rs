use std::path::PathBuf;

use serde::Serialize;
use yapstack_common::types::CaptureSource;

pub struct CapturedAudio {
    pub mic_samples: Vec<f32>,
    pub system_samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f32,
}

pub struct SessionMark {
    pub mic_write_pos: usize,
    pub system_write_pos: usize,
    pub started_at: std::time::Instant,
}

/// Lightweight cursor tracking for both ring buffers.
/// Used by the live transcription loop to track read positions independently of the session API.
#[derive(Debug, Clone, Copy, Default)]
pub struct BufferPositions {
    pub mic_pos: usize,
    pub system_pos: usize,
}

/// Separate per-source extraction from both ring buffers.
/// Each source is independently deinterleaved to mono.
pub struct SeparateExtraction {
    /// Mono samples and sample rate from the mic buffer, if available.
    pub mic: Option<(Vec<f32>, u32)>,
    /// Mono samples and sample rate from the system buffer, if available.
    pub system: Option<(Vec<f32>, u32)>,
    /// Updated buffer positions after extraction.
    pub new_positions: BufferPositions,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureResult {
    pub file_path: PathBuf,
    pub duration_seconds: f32,
    pub sample_rate: u32,
    pub source: CaptureSource,
}
