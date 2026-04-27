/// Lightweight cursor tracking for both ring buffers.
/// Used by the live transcription loop to track read positions independently
/// of the session API.
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
