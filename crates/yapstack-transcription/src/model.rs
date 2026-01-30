use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::error::TranscriptionError;

type Result<T> = std::result::Result<T, TranscriptionError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelSize {
    Tiny,
    Base,
    Small,
    Medium,
}

impl ModelSize {
    /// Returns the ggml model filename.
    pub fn filename(&self) -> &'static str {
        match self {
            ModelSize::Tiny => "ggml-tiny.bin",
            ModelSize::Base => "ggml-base.bin",
            ModelSize::Small => "ggml-small.bin",
            ModelSize::Medium => "ggml-medium.bin",
        }
    }

    /// Returns the approximate size in bytes.
    pub fn approximate_size_bytes(&self) -> u64 {
        match self {
            ModelSize::Tiny => 75_000_000,
            ModelSize::Base => 142_000_000,
            ModelSize::Small => 466_000_000,
            ModelSize::Medium => 1_500_000_000,
        }
    }

    /// Returns the Hugging Face download URL.
    pub fn download_url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.filename()
        )
    }

    /// Returns a human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            ModelSize::Tiny => "Tiny (~75 MB)",
            ModelSize::Base => "Base (~142 MB)",
            ModelSize::Small => "Small (~466 MB)",
            ModelSize::Medium => "Medium (~1.5 GB)",
        }
    }

    /// All available model sizes.
    pub fn all() -> &'static [ModelSize] {
        &[
            ModelSize::Tiny,
            ModelSize::Base,
            ModelSize::Small,
            ModelSize::Medium,
        ]
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub size: ModelSize,
    pub downloaded: bool,
    pub path: Option<PathBuf>,
    pub display_name: String,
    pub approximate_size_bytes: u64,
}

const VAD_MODEL_FILENAME: &str = "ggml-silero-v6.2.0.bin";
const VAD_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin";
const VAD_MODEL_SIZE_BYTES: u64 = 885_000;

/// Download a file from `url` to `dest` via a temp file, with streaming progress.
/// `fallback_size` is used when the server doesn't provide Content-Length.
async fn download_file(
    url: &str,
    dest: &Path,
    fallback_size: u64,
    on_progress: &(impl Fn(f32) + Send),
) -> Result<()> {
    let temp_dest = dest.with_extension("download");

    let client = reqwest::Client::new();
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(TranscriptionError::DownloadFailed(format!(
            "HTTP {} for {}",
            response.status(),
            url
        )));
    }

    let total_size = response.content_length().unwrap_or(fallback_size);

    let mut file = tokio::fs::File::create(&temp_dest).await?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    let stream_result: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| TranscriptionError::DownloadFailed(e.to_string()))?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            let progress = (downloaded as f32 / total_size as f32).min(1.0);
            on_progress(progress);
        }
        file.flush().await?;
        Ok(())
    }
    .await;

    if let Err(e) = stream_result {
        drop(file);
        let _ = tokio::fs::remove_file(&temp_dest).await;
        return Err(e);
    }

    drop(file);

    if let Err(e) = tokio::fs::rename(&temp_dest, dest).await {
        let _ = tokio::fs::remove_file(&temp_dest).await;
        return Err(e.into());
    }

    on_progress(1.0);
    Ok(())
}

#[derive(Clone)]
pub struct ModelManager {
    models_dir: PathBuf,
}

impl ModelManager {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let models_dir = app_data_dir.join("models");
        Self { models_dir }
    }

    /// Returns the directory where models are stored.
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// Check if a model is downloaded.
    pub fn is_available(&self, size: ModelSize) -> bool {
        self.model_path(size).is_some()
    }

    /// Get path to model file if it exists.
    pub fn model_path(&self, size: ModelSize) -> Option<PathBuf> {
        let path = self.models_dir.join(size.filename());
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Get path where a model would be stored (whether or not it exists).
    pub fn expected_model_path(&self, size: ModelSize) -> PathBuf {
        self.models_dir.join(size.filename())
    }

    /// Download model with progress callback.
    ///
    /// The callback receives a progress value from 0.0 to 1.0.
    pub async fn download(
        &self,
        size: ModelSize,
        on_progress: impl Fn(f32) + Send,
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.models_dir).await?;

        let url = size.download_url();
        let dest = self.models_dir.join(size.filename());

        info!("downloading model {} from {}", size.filename(), url);
        download_file(&url, &dest, size.approximate_size_bytes(), &on_progress).await?;
        info!("model {} downloaded successfully", size.filename());

        Ok(dest)
    }

    /// Verify model file checksum using streaming reads to avoid loading
    /// the entire file (potentially 1.5 GB+) into memory.
    pub async fn verify_checksum(&self, size: ModelSize, expected_sha256: &str) -> Result<bool> {
        use tokio::io::AsyncReadExt;

        let path = self
            .model_path(size)
            .ok_or_else(|| TranscriptionError::ModelNotFound(size.filename().to_string()))?;

        let file = tokio::fs::File::open(&path).await?;
        let mut reader = tokio::io::BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024]; // 64 KB chunks

        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }

        let result = format!("{:x}", hasher.finalize());

        if result != expected_sha256 {
            warn!(
                "checksum mismatch for {}: expected {}, got {} — deleting corrupted file",
                size.filename(),
                expected_sha256,
                result
            );
            let _ = tokio::fs::remove_file(&path).await;
            return Ok(false);
        }

        Ok(true)
    }

    /// Delete a downloaded model.
    pub async fn delete(&self, size: ModelSize) -> Result<()> {
        if let Some(path) = self.model_path(size) {
            tokio::fs::remove_file(&path).await?;
            info!("deleted model {}", size.filename());
        }
        Ok(())
    }

    /// List all downloaded models.
    pub fn list_downloaded(&self) -> Vec<ModelSize> {
        ModelSize::all()
            .iter()
            .copied()
            .filter(|s| self.is_available(*s))
            .collect()
    }

    /// Get info for all models (downloaded or not).
    pub fn list_all(&self) -> Vec<ModelInfo> {
        ModelSize::all()
            .iter()
            .map(|&size| ModelInfo {
                size,
                downloaded: self.is_available(size),
                path: self.model_path(size),
                display_name: size.display_name().to_string(),
                approximate_size_bytes: size.approximate_size_bytes(),
            })
            .collect()
    }

    /// Get path to VAD model file if it exists.
    pub fn vad_model_path(&self) -> Option<PathBuf> {
        let path = self.models_dir.join(VAD_MODEL_FILENAME);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Download the Silero VAD model with progress callback.
    pub async fn download_vad_model(&self, on_progress: impl Fn(f32) + Send) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.models_dir).await?;

        let dest = self.models_dir.join(VAD_MODEL_FILENAME);

        info!("downloading VAD model from {}", VAD_MODEL_URL);
        download_file(VAD_MODEL_URL, &dest, VAD_MODEL_SIZE_BYTES, &on_progress).await?;
        info!("VAD model downloaded successfully");

        Ok(dest)
    }

    /// Ensure the VAD model is available, downloading if missing.
    pub async fn ensure_vad_model(&self) -> Result<PathBuf> {
        if let Some(path) = self.vad_model_path() {
            return Ok(path);
        }
        self.download_vad_model(|_| {}).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_size_filenames() {
        assert_eq!(ModelSize::Tiny.filename(), "ggml-tiny.bin");
        assert_eq!(ModelSize::Base.filename(), "ggml-base.bin");
        assert_eq!(ModelSize::Small.filename(), "ggml-small.bin");
        assert_eq!(ModelSize::Medium.filename(), "ggml-medium.bin");
    }

    #[test]
    fn test_model_size_urls() {
        let url = ModelSize::Tiny.download_url();
        assert!(url.contains("huggingface.co"));
        assert!(url.contains("ggml-tiny.bin"));
    }

    #[test]
    fn test_model_size_all() {
        assert_eq!(ModelSize::all().len(), 4);
    }

    #[test]
    fn test_model_manager_not_available() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(!manager.is_available(ModelSize::Tiny));
        assert!(manager.model_path(ModelSize::Tiny).is_none());
    }

    #[test]
    fn test_model_manager_list_empty() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(manager.list_downloaded().is_empty());
    }

    #[test]
    fn test_model_manager_list_all() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        let all = manager.list_all();
        assert_eq!(all.len(), 4);
        assert!(all.iter().all(|m| !m.downloaded));
    }

    #[test]
    fn test_model_manager_detects_existing() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("ggml-tiny.bin"), b"fake model data").unwrap();

        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(manager.is_available(ModelSize::Tiny));
        assert!(!manager.is_available(ModelSize::Base));
        assert_eq!(manager.list_downloaded(), vec![ModelSize::Tiny]);
    }

    #[test]
    fn test_vad_model_not_available() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(manager.vad_model_path().is_none());
    }

    #[test]
    fn test_vad_model_detects_existing() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("ggml-silero-v6.2.0.bin"), b"fake vad model").unwrap();

        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(manager.vad_model_path().is_some());
    }

    #[tokio::test]
    async fn test_verify_checksum_deletes_on_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Write fake model data
        let model_path = models_dir.join(ModelSize::Tiny.filename());
        std::fs::write(&model_path, b"fake model data that will fail checksum").unwrap();
        assert!(model_path.exists());

        let manager = ModelManager::new(dir.path().to_path_buf());
        assert!(manager.is_available(ModelSize::Tiny));

        // Verify against a wrong hash — should return Ok(false) and delete the file
        let result = manager
            .verify_checksum(
                ModelSize::Tiny,
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .await
            .unwrap();
        assert!(!result);
        assert!(!model_path.exists(), "corrupted file should be deleted");
        assert!(!manager.is_available(ModelSize::Tiny));
    }

    #[test]
    fn test_model_size_serde_roundtrip() {
        for size in ModelSize::all() {
            let json = serde_json::to_string(size).unwrap();
            let deserialized: ModelSize = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, *size);
        }
    }
}
