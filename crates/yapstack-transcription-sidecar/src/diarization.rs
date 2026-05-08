//! Shared speaker-diarization post-pass.
//!
//! Sortformer runs after either Whisper or Parakeet has produced
//! [`TranscriptSegment`]s for an audio chunk. The dispatcher in `main.rs`
//! owns one [`DiarizationContext`] for the lifetime of the sidecar process
//! (== one session) and feeds every chunk through it via
//! [`apply_diarization`]. The streaming Sortformer API
//! ([`Sortformer::diarize_chunk`]) preserves its FIFO + speaker cache +
//! silence profile across calls, so speaker IDs stay stable across chunk
//! boundaries — that's the *only* reason this is a struct held at the
//! dispatcher level rather than a free function.

use std::path::PathBuf;

use parakeet_rs::sortformer::{Sortformer, SpeakerSegment};
use tracing::{info, warn};
use yapstack_common::types::TranscriptSegment;

/// Sortformer expects audio at exactly this sample rate (it errors otherwise).
/// Matches the rate the dispatcher resamples WAVs to before calling either
/// engine, so no extra resample step is needed in the post-pass.
const SORTFORMER_SAMPLE_RATE: u32 = 16_000;

/// Per-process diarization state. Holds the lazy-loaded Sortformer ONNX
/// session — constructed at first chunk that requests diarization, reused
/// for the rest of the process.
///
/// Stability of speaker IDs across chunks comes from `Sortformer`'s own
/// streaming state (FIFO + speaker cache). We never call `reset_state()`
/// — sidecar process exit handles that implicitly when a session ends.
pub struct DiarizationContext {
    model_path: Option<PathBuf>,
    sortformer: Option<Sortformer>,
}

impl DiarizationContext {
    /// `model_path = None` ⇒ diarization is permanently unavailable in this
    /// process (e.g. weights weren't downloaded yet at spawn). Every call to
    /// [`apply_diarization`] becomes a silent no-op.
    pub fn new(model_path: Option<PathBuf>) -> Self {
        Self {
            model_path,
            sortformer: None,
        }
    }

    /// Whether this context could produce speaker IDs if a chunk arrived now.
    /// Used by the dispatcher for telemetry / defense-in-depth checks before
    /// honoring `opts.diarization`.
    pub fn is_available(&self) -> bool {
        self.model_path.is_some()
    }

    fn ensure_loaded(&mut self) -> Result<&mut Sortformer, String> {
        if self.sortformer.is_none() {
            let path = self
                .model_path
                .as_ref()
                .ok_or("sortformer model not provided to sidecar")?;
            info!(
                "loading sortformer diarization model from {}",
                path.display()
            );
            let s = Sortformer::new(path)
                .map_err(|e| format!("failed to load sortformer model: {e}"))?;
            self.sortformer = Some(s);
        }
        Ok(self.sortformer.as_mut().unwrap())
    }
}

/// Run Sortformer on `audio` (16 kHz mono f32) and stamp each entry of
/// `segments` with the speaker ID that overlaps it the most. Segments with
/// no overlap are left with `speaker_id = None`. On any Sortformer failure
/// the function logs and returns `Ok(())` without touching `segments` —
/// missing speaker IDs are always a recoverable degradation, never a
/// transcription-blocking error.
///
/// Uses [`Sortformer::diarize_chunk`] (state-preserving) so consecutive
/// calls share the same FIFO and speaker cache; without that, the same
/// person flips between Speaker 0 / Speaker 1 across chunk boundaries.
pub fn apply_diarization(
    ctx: &mut DiarizationContext,
    audio: &[f32],
    segments: &mut [TranscriptSegment],
) {
    if segments.is_empty() {
        return;
    }
    if !ctx.is_available() {
        return;
    }
    let sortformer = match ctx.ensure_loaded() {
        Ok(s) => s,
        Err(e) => {
            warn!("diarization unavailable; segments will have no speaker_id: {e}");
            return;
        }
    };
    let speakers = match sortformer.diarize_chunk(audio) {
        Ok(s) => s,
        Err(e) => {
            warn!("diarization failed; returning transcript without speaker IDs: {e}");
            return;
        }
    };
    assign_speakers(segments, &speakers);
}

/// Stamp each transcript segment with the speaker_id that owns the largest
/// overlap, by milliseconds. Engine-agnostic: Whisper and Parakeet both
/// produce `start_ms`/`end_ms` fields, so this works on either.
fn assign_speakers(segments: &mut [TranscriptSegment], speakers: &[SpeakerSegment]) {
    for seg in segments {
        let mut best: Option<(u64, u8)> = None;
        for sp in speakers {
            let sp_start_ms = sp.start * 1000 / SORTFORMER_SAMPLE_RATE as u64;
            let sp_end_ms = sp.end * 1000 / SORTFORMER_SAMPLE_RATE as u64;
            let overlap_start = seg.start_ms.max(sp_start_ms);
            let overlap_end = seg.end_ms.min(sp_end_ms);
            if overlap_end > overlap_start {
                let overlap = overlap_end - overlap_start;
                if best.is_none_or(|(b, _)| overlap > b) {
                    best = Some((overlap, sp.speaker_id as u8));
                }
            }
        }
        if let Some((_, id)) = best {
            seg.speaker_id = Some(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(start_ms: u64, end_ms: u64) -> TranscriptSegment {
        TranscriptSegment {
            start_ms,
            end_ms,
            text: "x".to_string(),
            confidence: 1.0,
            speaker_id: None,
        }
    }

    fn sp(start_secs: f32, end_secs: f32, id: usize) -> SpeakerSegment {
        SpeakerSegment {
            start: (start_secs * SORTFORMER_SAMPLE_RATE as f32) as u64,
            end: (end_secs * SORTFORMER_SAMPLE_RATE as f32) as u64,
            speaker_id: id,
        }
    }

    #[test]
    fn assigns_speaker_with_full_overlap() {
        let mut segs = vec![ts(0, 1000), ts(2000, 3000)];
        let speakers = vec![sp(0.0, 1.5, 0), sp(1.6, 3.2, 1)];
        assign_speakers(&mut segs, &speakers);
        assert_eq!(segs[0].speaker_id, Some(0));
        assert_eq!(segs[1].speaker_id, Some(1));
    }

    #[test]
    fn assigns_speaker_with_max_overlap_when_segments_straddle() {
        let mut segs = vec![ts(0, 2000)];
        let speakers = vec![sp(0.0, 0.5, 0), sp(0.5, 2.0, 1)];
        assign_speakers(&mut segs, &speakers);
        assert_eq!(segs[0].speaker_id, Some(1));
    }

    #[test]
    fn leaves_speaker_none_when_no_overlap() {
        let mut segs = vec![ts(5000, 6000)];
        let speakers = vec![sp(0.0, 1.0, 0)];
        assign_speakers(&mut segs, &speakers);
        assert_eq!(segs[0].speaker_id, None);
    }

    #[test]
    fn assign_speakers_with_empty_diarization_is_noop() {
        let mut segs = vec![ts(0, 1000), ts(1000, 2000)];
        assign_speakers(&mut segs, &[]);
        assert!(segs.iter().all(|s| s.speaker_id.is_none()));
    }

    #[test]
    fn unavailable_context_is_noop() {
        let mut ctx = DiarizationContext::new(None);
        assert!(!ctx.is_available());
        let mut segs = vec![ts(0, 1000)];
        apply_diarization(&mut ctx, &[0.0_f32; 16_000], &mut segs);
        assert_eq!(segs[0].speaker_id, None);
    }

    #[test]
    fn empty_segments_is_noop_even_with_loaded_context() {
        let mut ctx = DiarizationContext::new(Some(PathBuf::from("/nonexistent")));
        let mut segs: Vec<TranscriptSegment> = vec![];
        apply_diarization(&mut ctx, &[0.0_f32; 16_000], &mut segs);
        assert!(segs.is_empty());
    }
}
