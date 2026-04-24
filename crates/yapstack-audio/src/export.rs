use std::fs::File;
use std::io::{BufWriter, Write};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};
use mp3lame_encoder::{Bitrate, Builder as LameBuilder, FlushNoGap, InterleavedPcm, MonoPcm};
use tempfile::Builder;

use crate::error::AudioError;

/// Command-boundary validator: returns `Ok(())` for a valid MP3 bitrate,
/// `Err(AudioError::InvalidBitrate)` otherwise. Callers that accept a user
/// bitrate should invoke this *before* doing any export work so a rejection
/// doesn't leave a half-written WAV on disk or consume the session's audio.
pub fn validate_mp3_bitrate(kbps: u16) -> Result<()> {
    mp3_bitrate_from_kbps(kbps).map(|_| ())
}

/// Maps a user-supplied MP3 bitrate (kbps) to the exact `Bitrate` enum variant.
/// Rejects any kbps value not matching a valid MP3 bitrate, so callers cannot
/// smuggle arbitrary u16 values into the LAME encoder.
fn mp3_bitrate_from_kbps(kbps: u16) -> Result<Bitrate> {
    Ok(match kbps {
        8 => Bitrate::Kbps8,
        16 => Bitrate::Kbps16,
        24 => Bitrate::Kbps24,
        32 => Bitrate::Kbps32,
        40 => Bitrate::Kbps40,
        48 => Bitrate::Kbps48,
        64 => Bitrate::Kbps64,
        80 => Bitrate::Kbps80,
        96 => Bitrate::Kbps96,
        112 => Bitrate::Kbps112,
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        224 => Bitrate::Kbps224,
        256 => Bitrate::Kbps256,
        320 => Bitrate::Kbps320,
        _ => return Err(AudioError::InvalidBitrate(kbps)),
    })
}

type Result<T> = std::result::Result<T, AudioError>;

/// Incrementally writes mono 16-bit PCM audio to a WAV file.
///
/// Designed for streaming session recording: call `write_samples()` periodically
/// to append audio, then `finalize()` to flush and close the file. The WAV header
/// is updated on `finalize()` to reflect the total data size.
pub struct SessionWavWriter {
    writer: WavWriter<BufWriter<File>>,
    path: PathBuf,
    sample_rate: u32,
    samples_written: u64,
}

impl SessionWavWriter {
    /// Creates a new WAV file at `path` for mono 16-bit PCM at the given sample rate.
    pub fn new(path: PathBuf, sample_rate: u32) -> Result<Self> {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let writer = WavWriter::create(&path, spec)?;
        Ok(Self {
            writer,
            path,
            sample_rate,
            samples_written: 0,
        })
    }

    /// Appends mono f32 samples (clamped to [-1.0, 1.0] and converted to i16).
    pub fn write_samples(&mut self, samples: &[f32]) -> Result<()> {
        for &sample in samples {
            self.writer.write_sample(f32_to_i16(sample))?;
        }
        self.samples_written += samples.len() as u64;
        Ok(())
    }

    /// Flushes the WAV, converts to MP3 at the given bitrate, deletes the WAV.
    /// Returns `(mp3_path, duration_seconds)`.
    pub fn finalize_as_mp3(self, bitrate_kbps: u16) -> Result<(PathBuf, f32)> {
        let duration = self.duration_seconds();
        self.writer.finalize()?;
        let mp3_path = convert_wav_to_mp3(&self.path, bitrate_kbps)?;
        Ok((mp3_path, duration))
    }

    /// Flushes and closes the WAV without MP3 conversion.
    /// Use when the file will be deleted immediately (e.g. zero-samples case).
    pub fn finalize_wav_only(self) -> Result<(PathBuf, f32)> {
        let duration = self.duration_seconds();
        self.writer.finalize()?;
        Ok((self.path, duration))
    }

    /// Returns the WAV file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the total number of samples written so far.
    pub fn samples_written(&self) -> u64 {
        self.samples_written
    }

    /// Returns the duration of audio written so far in seconds.
    pub fn duration_seconds(&self) -> f32 {
        self.samples_written as f32 / self.sample_rate as f32
    }
}

fn f32_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    (clamped * i16::MAX as f32) as i16
}

/// Writes f32 audio samples to a WAV file as 16-bit signed PCM.
pub fn write_wav(samples: &[f32], sample_rate: u32, channels: u16, path: &Path) -> Result<()> {
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut writer = WavWriter::create(path, spec)?;

    for &sample in samples {
        writer.write_sample(f32_to_i16(sample))?;
    }

    writer.finalize()?;
    Ok(())
}

/// Converts a WAV file to CBR MP3 at the given bitrate (kbps),
/// deletes the original WAV, and returns the MP3 path.
pub fn convert_wav_to_mp3(wav_path: &Path, bitrate_kbps: u16) -> Result<PathBuf> {
    let mut reader =
        hound::WavReader::open(wav_path).map_err(|e| AudioError::Mp3Encode(e.to_string()))?;
    let spec = reader.spec();

    let mut encoder = LameBuilder::new()
        .ok_or_else(|| AudioError::Mp3Encode("failed to create LAME encoder".into()))?;
    encoder
        .set_sample_rate(spec.sample_rate)
        .map_err(|e| AudioError::Mp3Encode(format!("set_sample_rate: {e:?}")))?;
    encoder
        .set_num_channels(spec.channels as u8)
        .map_err(|e| AudioError::Mp3Encode(format!("set_num_channels: {e:?}")))?;
    let bitrate = mp3_bitrate_from_kbps(bitrate_kbps)?;
    encoder
        .set_brate(bitrate)
        .map_err(|e| AudioError::Mp3Encode(format!("set_brate: {e:?}")))?;
    encoder
        .set_quality(mp3lame_encoder::Quality::Good)
        .map_err(|e| AudioError::Mp3Encode(format!("set_quality: {e:?}")))?;

    let mut encoder = encoder
        .build()
        .map_err(|e| AudioError::Mp3Encode(format!("build: {e:?}")))?;

    let mp3_path = wav_path.with_extension("mp3");
    let mut mp3_file =
        BufWriter::new(File::create(&mp3_path).map_err(|e| AudioError::Mp3Encode(e.to_string()))?);

    const CHUNK_FRAMES: usize = 8192;
    let samples_per_chunk = CHUNK_FRAMES * spec.channels as usize;
    let max_buf = mp3lame_encoder::max_required_buffer_size(samples_per_chunk);
    let mut mp3_buf: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); max_buf];
    let mut sample_buf: Vec<i16> = Vec::with_capacity(samples_per_chunk);

    let flush_chunk = |sample_buf: &mut Vec<i16>,
                       encoder: &mut mp3lame_encoder::Encoder,
                       mp3_buf: &mut Vec<MaybeUninit<u8>>,
                       mp3_file: &mut BufWriter<File>|
     -> Result<()> {
        if sample_buf.is_empty() {
            return Ok(());
        }
        let bytes_written = if spec.channels == 1 {
            encoder
                .encode(MonoPcm(sample_buf), mp3_buf)
                .map_err(|e| AudioError::Mp3Encode(format!("encode: {e:?}")))?
        } else {
            encoder
                .encode(InterleavedPcm(sample_buf), mp3_buf)
                .map_err(|e| AudioError::Mp3Encode(format!("encode: {e:?}")))?
        };
        let written =
            unsafe { std::slice::from_raw_parts(mp3_buf.as_ptr().cast::<u8>(), bytes_written) };
        mp3_file
            .write_all(written)
            .map_err(|e| AudioError::Mp3Encode(e.to_string()))?;
        sample_buf.clear();
        Ok(())
    };

    match spec.sample_format {
        SampleFormat::Int => {
            for sample in reader.samples::<i16>() {
                let s = sample.map_err(|e| AudioError::Mp3Encode(e.to_string()))?;
                sample_buf.push(s);
                if sample_buf.len() >= samples_per_chunk {
                    flush_chunk(&mut sample_buf, &mut encoder, &mut mp3_buf, &mut mp3_file)?;
                }
            }
        }
        SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                let s = sample.map_err(|e| AudioError::Mp3Encode(e.to_string()))?;
                sample_buf.push(f32_to_i16(s));
                if sample_buf.len() >= samples_per_chunk {
                    flush_chunk(&mut sample_buf, &mut encoder, &mut mp3_buf, &mut mp3_file)?;
                }
            }
        }
    }
    flush_chunk(&mut sample_buf, &mut encoder, &mut mp3_buf, &mut mp3_file)?;

    let mut flush_buf: Vec<MaybeUninit<u8>> =
        vec![MaybeUninit::uninit(); mp3lame_encoder::max_required_buffer_size(0)];
    let flush_bytes = encoder
        .flush::<FlushNoGap>(&mut flush_buf)
        .map_err(|e| AudioError::Mp3Encode(format!("flush: {e:?}")))?;
    let flushed =
        unsafe { std::slice::from_raw_parts(flush_buf.as_ptr().cast::<u8>(), flush_bytes) };
    mp3_file
        .write_all(flushed)
        .map_err(|e| AudioError::Mp3Encode(e.to_string()))?;
    drop(mp3_file);

    std::fs::remove_file(wav_path).map_err(|e| AudioError::Mp3Encode(e.to_string()))?;

    Ok(mp3_path)
}

/// Writes f32 audio samples to a temporary WAV file (16-bit signed PCM).
///
/// The file is persisted (not deleted on drop) so it outlives this function call.
/// The caller is responsible for cleanup.
pub fn write_wav_to_temp(samples: &[f32], sample_rate: u32, channels: u16) -> Result<PathBuf> {
    let temp_file = Builder::new()
        .prefix("yapstack_capture_")
        .suffix(".wav")
        .tempfile()?;

    // Write WAV *before* calling keep() — if writing fails, the NamedTempFile
    // handle will clean up the temp file on drop automatically.
    write_wav(samples, sample_rate, channels, temp_file.path())?;

    let (_, path) = temp_file
        .keep()
        .map_err(|e| AudioError::WavExport(e.to_string()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::WavReader;
    use std::fs;

    #[test]
    fn test_write_wav_creates_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let samples = vec![0.0f32; 16000]; // 1 second of silence at 16kHz

        write_wav(&samples, 16000, 1, &path).unwrap();

        let reader = WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, SampleFormat::Int);
        assert_eq!(reader.len(), 16000);
    }

    #[test]
    fn test_roundtrip_sample_verification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_roundtrip.wav");
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0];

        write_wav(&samples, 16000, 1, &path).unwrap();

        let mut reader = WavReader::open(&path).unwrap();
        let read_samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(read_samples.len(), 5);
        assert_eq!(read_samples[0], 0);
        // 0.5 * 32767 ≈ 16383
        assert!((read_samples[1] - 16383).abs() <= 1);
        assert!((read_samples[2] + 16383).abs() <= 1);
        assert_eq!(read_samples[3], i16::MAX);
        // -1.0 * 32767 = -32767
        assert!((read_samples[4] + i16::MAX).abs() <= 1);
    }

    #[test]
    fn test_temp_file_creation() {
        let samples = vec![0.0f32; 1600];
        let path = write_wav_to_temp(&samples, 16000, 1).unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().contains("yapstack_capture_"));
        assert!(path.extension().is_some_and(|ext| ext == "wav"));

        // Cleanup
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_clamping_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_clamp.wav");
        let samples = vec![2.0, -3.0, 1.5, -1.5];

        write_wav(&samples, 16000, 1, &path).unwrap();

        let mut reader = WavReader::open(&path).unwrap();
        let read_samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        // Values should be clamped to i16::MAX / i16::MIN range
        assert_eq!(read_samples[0], i16::MAX);
        assert!((read_samples[1] + i16::MAX).abs() <= 1);
        assert_eq!(read_samples[2], i16::MAX);
        assert!((read_samples[3] + i16::MAX).abs() <= 1);
    }

    // --- SessionWavWriter tests ---

    #[test]
    fn test_session_wav_writer_finalizes_to_mp3() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.wav");

        let mut writer = SessionWavWriter::new(path.clone(), 48000).unwrap();
        let samples = vec![0.0f32; 48000]; // 1 second at 48kHz
        writer.write_samples(&samples).unwrap();
        let (result_path, duration) = writer.finalize_as_mp3(64).unwrap();

        assert_eq!(result_path.extension().unwrap(), "mp3");
        assert!(!path.exists(), "WAV should be deleted after conversion");
        assert!(result_path.exists(), "MP3 should exist");
        assert!((duration - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_session_wav_writer_finalize_wav_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session_raw.wav");

        let mut writer = SessionWavWriter::new(path.clone(), 48000).unwrap();
        let samples = vec![0.0f32; 48000];
        writer.write_samples(&samples).unwrap();
        let (result_path, duration) = writer.finalize_wav_only().unwrap();

        assert_eq!(result_path, path);
        assert!(path.exists(), "WAV should still exist");
        assert!((duration - 1.0).abs() < 0.001);

        let reader = WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 48000);
        assert_eq!(reader.len(), 48000);
    }

    #[test]
    fn test_session_wav_writer_incremental_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("incremental.wav");

        let mut writer = SessionWavWriter::new(path.clone(), 16000).unwrap();
        let chunk = vec![0.5f32; 8000];
        writer.write_samples(&chunk).unwrap();
        assert!((writer.duration_seconds() - 0.5).abs() < 0.001);
        writer.write_samples(&chunk).unwrap();
        assert!((writer.duration_seconds() - 1.0).abs() < 0.001);
        writer.write_samples(&chunk).unwrap();

        let (result_path, duration) = writer.finalize_as_mp3(64).unwrap();
        assert!((duration - 1.5).abs() < 0.001);
        assert_eq!(result_path.extension().unwrap(), "mp3");
        assert!(!path.exists());
    }

    #[test]
    fn test_session_wav_writer_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wav");

        let writer = SessionWavWriter::new(path.clone(), 48000).unwrap();
        let (_, duration) = writer.finalize_wav_only().unwrap();
        assert!((duration - 0.0).abs() < f32::EPSILON);

        let reader = WavReader::open(&path).unwrap();
        assert_eq!(reader.len(), 0);
    }

    // --- MP3 conversion tests ---

    #[test]
    fn test_convert_wav_to_mp3() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("test.wav");
        let samples = vec![0.0f32; 16000]; // 1s at 16kHz
        write_wav(&samples, 16000, 1, &wav_path).unwrap();

        let mp3_path = convert_wav_to_mp3(&wav_path, 64).unwrap();

        assert_eq!(mp3_path, dir.path().join("test.mp3"));
        assert!(mp3_path.exists());
        assert!(!wav_path.exists(), "original WAV should be deleted");
        assert!(fs::metadata(&mp3_path).unwrap().len() > 0);
    }

    #[test]
    fn test_convert_wav_to_mp3_rejects_invalid_bitrate() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("invalid_bitrate.wav");
        let samples = vec![0.0f32; 16000];
        write_wav(&samples, 16000, 1, &wav_path).unwrap();

        // 100 kbps is not a valid MP3 bitrate.
        let err = convert_wav_to_mp3(&wav_path, 100).unwrap_err();
        assert!(matches!(err, AudioError::InvalidBitrate(100)));
    }

    #[test]
    fn test_mp3_bitrate_mapping_accepts_all_valid_kbps() {
        for kbps in [
            8, 16, 24, 32, 40, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ] {
            assert!(
                mp3_bitrate_from_kbps(kbps).is_ok(),
                "{kbps} kbps should be accepted"
            );
        }
    }

    #[test]
    fn test_mp3_bitrate_mapping_rejects_invalid_kbps() {
        for kbps in [0, 1, 100, 200, 321, 1000] {
            assert!(
                matches!(
                    mp3_bitrate_from_kbps(kbps),
                    Err(AudioError::InvalidBitrate(_))
                ),
                "{kbps} kbps should be rejected"
            );
        }
    }

    #[test]
    fn test_convert_wav_to_mp3_smaller_than_wav() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("size_test.wav");
        // 5 seconds of audio
        let samples: Vec<f32> = (0..80000).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        write_wav(&samples, 16000, 1, &wav_path).unwrap();
        let wav_size = fs::metadata(&wav_path).unwrap().len();

        let mp3_path = convert_wav_to_mp3(&wav_path, 64).unwrap();
        let mp3_size = fs::metadata(&mp3_path).unwrap().len();

        assert!(
            mp3_size < wav_size / 2,
            "MP3 ({mp3_size}) should be significantly smaller than WAV ({wav_size})"
        );
    }
}
