use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use yapstack_common::types::{EngineKind, SidecarRequest, SidecarResponse, TranscriptSegment};

use crate::error::TranscriptionError;

type Result<T> = std::result::Result<T, TranscriptionError>;

/// Remove ANSI CSI / OSC escape sequences. Tiny state machine — avoids pulling
/// in a dependency for one place that needs it. Single-pass over the chars().
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1B' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            // CSI: consume until the final byte in @..~ (0x40..=0x7E).
            Some('[') => {
                for c in chars.by_ref() {
                    if matches!(c, '\x40'..='\x7E') {
                        break;
                    }
                }
            }
            // OSC: consume until BEL or `ESC \`.
            Some(']') => {
                let mut prev_esc = false;
                for c in chars.by_ref() {
                    if c == '\x07' || (prev_esc && c == '\\') {
                        break;
                    }
                    prev_esc = c == '\x1B';
                }
            }
            // Other 2-byte escapes or trailing ESC: drop both.
            _ => {}
        }
    }
    out
}

/// Log a sidecar response at DEBUG without exposing user speech — Transcription
/// variants carry full segment text, so we never `Debug`-print the value.
fn log_sidecar_response(response: &SidecarResponse) {
    match response {
        SidecarResponse::Transcription {
            id,
            segments,
            duration_ms,
            ..
        } => debug!(
            id = id,
            segments = segments.len(),
            duration_ms = duration_ms,
            "sidecar response: transcription"
        ),
        SidecarResponse::ModelLoaded { id } => {
            debug!(id = id, "sidecar response: model_loaded")
        }
        SidecarResponse::Error { id, message } => debug!(
            id = id,
            message = message.as_str(),
            "sidecar response: error"
        ),
        SidecarResponse::Progress { id, percent } => {
            debug!(id = id, percent = percent, "sidecar response: progress")
        }
    }
}

#[cfg(test)]
mod strip_ansi_tests {
    use super::strip_ansi;

    #[test]
    fn strips_sgr_colors() {
        let input = "\x1b[32mINFO\x1b[0m hello \x1b[2mworld\x1b[0m";
        assert_eq!(strip_ansi(input), "INFO hello world");
    }

    #[test]
    fn preserves_plain() {
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }

    #[test]
    fn preserves_unicode() {
        assert_eq!(strip_ansi("héllo — world"), "héllo — world");
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
    pub duration_ms: u64,
}

pub struct TranscriptionClient {
    stdin: ChildStdin,
    response_rx: mpsc::Receiver<SidecarResponse>,
    next_id: u64,
    _child: Child,
    // Stored for auto-restart (respawn)
    sidecar_path: PathBuf,
    engine: EngineKind,
    model_path: PathBuf,
    vad_model_path: Option<PathBuf>,
    sortformer_model_path: Option<PathBuf>,
    coreml_cache_dir: Option<PathBuf>,
}

/// Raw parts returned by sidecar spawn (avoids Drop issues when moving into an existing TranscriptionClient).
struct SidecarParts {
    stdin: ChildStdin,
    response_rx: mpsc::Receiver<SidecarResponse>,
    child: Child,
}

impl TranscriptionClient {
    /// Spawns the sidecar process and sets up JSON-line IPC.
    ///
    /// `engine` selects which backend the sidecar instantiates (Whisper or
    /// Parakeet). `vad_model_path` is honored only for Whisper (Silero VAD,
    /// reduces hallucinations). `sortformer_model_path` is honored only for
    /// Parakeet — when set, per-request `diarization: true` populates speaker IDs.
    /// `coreml_cache_dir` is Parakeet+macOS only; persistent cache for compiled
    /// CoreML graphs to avoid the ~5 s recompile on every spawn.
    pub async fn spawn(
        sidecar_path: &Path,
        engine: EngineKind,
        model_path: &Path,
        vad_model_path: Option<&Path>,
        sortformer_model_path: Option<&Path>,
        coreml_cache_dir: Option<&Path>,
    ) -> Result<Self> {
        let parts = Self::spawn_sidecar(
            sidecar_path,
            engine,
            model_path,
            vad_model_path,
            sortformer_model_path,
            coreml_cache_dir,
        )
        .await?;

        Ok(Self {
            stdin: parts.stdin,
            response_rx: parts.response_rx,
            next_id: 1,
            _child: parts.child,
            sidecar_path: sidecar_path.to_path_buf(),
            engine,
            model_path: model_path.to_path_buf(),
            vad_model_path: vad_model_path.map(|p| p.to_path_buf()),
            sortformer_model_path: sortformer_model_path.map(|p| p.to_path_buf()),
            coreml_cache_dir: coreml_cache_dir.map(|p| p.to_path_buf()),
        })
    }

    /// Internal: spawn the sidecar process and return raw parts.
    async fn spawn_sidecar(
        sidecar_path: &Path,
        engine: EngineKind,
        model_path: &Path,
        vad_model_path: Option<&Path>,
        sortformer_model_path: Option<&Path>,
        coreml_cache_dir: Option<&Path>,
    ) -> Result<SidecarParts> {
        info!(
            "spawning sidecar: {} engine={} model={} vad_model={:?} sortformer_model={:?} coreml_cache_dir={:?}",
            sidecar_path.display(),
            engine.as_str(),
            model_path.display(),
            vad_model_path.map(|p| p.display().to_string()),
            sortformer_model_path.map(|p| p.display().to_string()),
            coreml_cache_dir.map(|p| p.display().to_string()),
        );

        let mut cmd = tokio::process::Command::new(sidecar_path);
        cmd.arg("--engine").arg(engine.as_str());
        cmd.arg("--model").arg(model_path);

        if let Some(vad_path) = vad_model_path {
            cmd.arg("--vad-model").arg(vad_path);
        }
        if let Some(s_path) = sortformer_model_path {
            cmd.arg("--sortformer-model").arg(s_path);
        }
        if let Some(c_path) = coreml_cache_dir {
            cmd.arg("--coreml-cache-dir").arg(c_path);
        }

        // On Windows, prevent the sidecar from creating a visible console window.
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TranscriptionError::SidecarError("failed to open stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TranscriptionError::SidecarError("failed to open stdout".to_string()))?;
        let stderr = child.stderr.take();

        let (tx, rx) = mpsc::channel::<SidecarResponse>(64);

        // Spawn reader task to read JSON lines from stdout
        tokio::spawn(Self::reader_task(stdout, tx));

        // Spawn stderr reader task to forward sidecar logs to tracing
        if let Some(stderr) = stderr {
            tokio::spawn(Self::stderr_reader_task(stderr));
        }

        Ok(SidecarParts {
            stdin,
            response_rx: rx,
            child,
        })
    }

    async fn reader_task(stdout: ChildStdout, tx: mpsc::Sender<SidecarResponse>) {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<SidecarResponse>(&line) {
                        Ok(response) => {
                            log_sidecar_response(&response);
                            if tx.send(response).await.is_err() {
                                debug!("response channel closed, stopping reader");
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("failed to parse sidecar response: {}: {}", e, line);
                        }
                    }
                }
                Ok(None) => {
                    info!("sidecar stdout closed");
                    break;
                }
                Err(e) => {
                    error!("error reading sidecar stdout: {}", e);
                    break;
                }
            }
        }
    }

    /// Reads stderr from the sidecar and forwards each line to tracing.
    /// Strips ANSI escape sequences defensively — the sidecar itself disables
    /// ANSI but native libraries (ORT, Dawn, whisper.cpp) may still emit
    /// colored output that would render as boxes in the desktop log UI.
    async fn stderr_reader_task(stderr: tokio::process::ChildStderr) {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let stripped = strip_ansi(&line);
                    if !stripped.trim().is_empty() {
                        info!(target: "yapstack_sidecar", "{}", stripped);
                    }
                }
                Ok(None) => {
                    debug!("sidecar stderr closed");
                    break;
                }
                Err(e) => {
                    warn!("error reading sidecar stderr: {}", e);
                    break;
                }
            }
        }
    }

    /// Sends a request to the sidecar process.
    async fn send_request(&mut self, request: &SidecarRequest) -> Result<()> {
        let mut json = serde_json::to_string(request)?;
        json.push('\n');
        self.stdin.write_all(json.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Waits for a response with the given ID, with timeout.
    async fn wait_for_response(&mut self, id: u64, timeout_secs: u64) -> Result<SidecarResponse> {
        let timeout = tokio::time::Duration::from_secs(timeout_secs);

        loop {
            match tokio::time::timeout(timeout, self.response_rx.recv()).await {
                Ok(Some(response)) => {
                    let response_id = match &response {
                        SidecarResponse::Transcription { id, .. } => Some(*id),
                        SidecarResponse::ModelLoaded { id } => Some(*id),
                        SidecarResponse::Error { id, .. } => Some(*id),
                        SidecarResponse::Progress { id, .. } => Some(*id),
                    };

                    if response_id == Some(id) {
                        // Skip progress messages — keep waiting for the final result
                        if let SidecarResponse::Progress { percent, .. } = &response {
                            debug!("transcription progress: {:.0}%", percent * 100.0);
                            continue;
                        }
                        return Ok(response);
                    } else {
                        warn!(
                            "IPC response ID mismatch: expected {}, got {:?} — discarding",
                            id, response_id
                        );
                    }
                }
                Ok(None) => {
                    return Err(TranscriptionError::SidecarError(
                        "sidecar process exited unexpectedly".to_string(),
                    ));
                }
                Err(_) => {
                    return Err(TranscriptionError::Timeout(timeout_secs));
                }
            }
        }
    }

    /// Engine the sidecar was spawned with. Read-only after construction.
    pub fn engine(&self) -> EngineKind {
        self.engine
    }

    /// Transcribes an audio file. Diarization is off — use
    /// [`Self::transcribe_with`] when you need to control it per-call.
    pub async fn transcribe(
        &mut self,
        audio_path: &Path,
        language: Option<&str>,
        initial_prompt: Option<&str>,
    ) -> Result<TranscriptionResult> {
        self.transcribe_with(audio_path, language, initial_prompt, false)
            .await
    }

    /// Transcribes an audio file with explicit per-request diarization control.
    /// `diarization: true` is honored only when the sidecar was spawned with
    /// `engine = Parakeet` *and* a Sortformer model path; otherwise the flag
    /// is silently a no-op.
    pub async fn transcribe_with(
        &mut self,
        audio_path: &Path,
        language: Option<&str>,
        initial_prompt: Option<&str>,
        diarization: bool,
    ) -> Result<TranscriptionResult> {
        let id = self.next_id;
        self.next_id += 1;

        let request = SidecarRequest::Transcribe {
            id,
            audio_path: audio_path.to_path_buf(),
            language: language.map(String::from),
            initial_prompt: initial_prompt.map(String::from),
            single_segment: None, // let sidecar decide based on audio duration
            diarization,
        };

        self.send_request(&request).await?;

        // 5 minutes timeout for transcription (long audio files)
        let response = self.wait_for_response(id, 300).await?;

        match response {
            SidecarResponse::Transcription {
                text,
                segments,
                duration_ms,
                ..
            } => Ok(TranscriptionResult {
                text,
                segments,
                duration_ms,
            }),
            SidecarResponse::Error { message, .. } => {
                Err(TranscriptionError::TranscriptionFailed(message))
            }
            _ => Err(TranscriptionError::SidecarError(
                "unexpected response type".to_string(),
            )),
        }
    }

    /// Loads a model into the sidecar process.
    pub async fn load_model(&mut self, model_path: &Path) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;

        let request = SidecarRequest::LoadModel {
            id,
            model_path: model_path.to_path_buf(),
        };

        self.send_request(&request).await?;

        // 60 seconds timeout for model loading
        let response = self.wait_for_response(id, 60).await?;

        match response {
            SidecarResponse::ModelLoaded { .. } => Ok(()),
            SidecarResponse::Error { message, .. } => {
                Err(TranscriptionError::SidecarError(message))
            }
            _ => Err(TranscriptionError::SidecarError(
                "unexpected response type".to_string(),
            )),
        }
    }

    /// Sends a shutdown request and waits for the process to exit.
    pub async fn shutdown(&mut self) -> Result<()> {
        let request = SidecarRequest::Shutdown;
        // Best-effort send; if it fails, the process may already be dead
        let _ = self.send_request(&request).await;
        Ok(())
    }

    /// Returns whether the sidecar process is likely still running.
    ///
    /// Combines two checks:
    /// 1. Response channel open (reader task / stdout still active)
    /// 2. Child process hasn't exited (via `try_wait`)
    pub fn is_running(&mut self) -> bool {
        if self.response_rx.is_closed() {
            return false;
        }
        match self._child.try_wait() {
            Ok(Some(status)) => {
                warn!("sidecar process exited with status: {}", status);
                false
            }
            Ok(None) => true, // still running
            Err(e) => {
                warn!("failed to check sidecar process status: {}", e);
                // Assume running if we can't check — channel state is the fallback
                true
            }
        }
    }

    /// Kills the current sidecar and spawns a fresh one with the same
    /// model/VAD configuration. The model is loaded eagerly by the sidecar
    /// on startup (via --model arg), so no separate load_model call is needed.
    /// `next_id` is preserved to keep request IDs unique across respawns.
    pub async fn respawn(&mut self) -> Result<()> {
        // Log the old process's exit status for diagnostics
        match self._child.try_wait() {
            Ok(Some(status)) => info!(
                "respawning sidecar (previous exited with status: {})",
                status
            ),
            Ok(None) => info!("respawning sidecar (previous still running — will kill)"),
            Err(e) => info!(
                "respawning sidecar (could not check previous status: {})",
                e
            ),
        }

        let _ = self._child.start_kill();

        let parts = Self::spawn_sidecar(
            &self.sidecar_path,
            self.engine,
            &self.model_path,
            self.vad_model_path.as_deref(),
            self.sortformer_model_path.as_deref(),
            self.coreml_cache_dir.as_deref(),
        )
        .await?;

        self.stdin = parts.stdin;
        self.response_rx = parts.response_rx;
        self._child = parts.child;
        // Don't reset next_id — keep incrementing for unique request IDs
        info!(
            "sidecar respawned successfully (next request id: {})",
            self.next_id
        );
        Ok(())
    }
}

impl Drop for TranscriptionClient {
    fn drop(&mut self) {
        // Kill the sidecar process to prevent orphaned processes when the
        // client is dropped without an explicit shutdown() call.
        let _ = self._child.start_kill();
    }
}
