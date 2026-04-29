//! Transcription scheduler: a single-worker priority queue in front of the
//! sidecar lane.
//!
//! The sidecar processes requests serially on the model side; two concurrent
//! `transcribe_with` calls both land in the sidecar's stdin queue in FIFO
//! order, with no way to prefer fresh live speech over historical backfill.
//! The scheduler fixes that: every transcription is submitted as a `Job` with
//! a priority (`FinalFlush > Live > Backfill`). A single worker task picks
//! the highest-priority job available, calls `transcribe_with`, and responds
//! via a per-job oneshot. Within the `Live` priority, mic/system sources
//! alternate in strict round-robin so neither can starve the other during a
//! sustained dual-source session.
//!
//! Sidecar liveness: because the worker is the *sole* caller of
//! `transcribe_with`, a transient sidecar error can be resolved cleanly — the
//! worker owns the only live `Arc<TranscriptionClient>` clone outside of its
//! own brief in-call clone, so `Arc::try_unwrap` succeeds at `respawn()` time.
//!
//! Backfill durability: on stop, the live loop submits any in-flight speech
//! as `FinalFlush` jobs, then waits for the backfill submitter task to finish
//! enqueuing chunks, then calls `shutdown_and_return`. The worker drains the
//! whole queue (FinalFlush + remaining Backfill) before exiting, so backfill
//! audio that was already extracted into memory is never silently dropped on
//! stop. The shutdown timeout is the only ceiling — generous (5 min) so a
//! long backfill can finish, but bounded so a wedged sidecar can't hang the
//! stop path forever.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::{oneshot, Mutex as TokioMutex, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use super::transcription::TranscriptionClientState;

/// Origin of a transcription job — mirrors the `origin` field emitted on
/// `LiveSegmentEvent` so the frontend can bucket segments by priority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobOrigin {
    Live,
    Backfill,
    FinalFlush,
}

impl JobOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            JobOrigin::Live => "live",
            JobOrigin::Backfill => "backfill",
            JobOrigin::FinalFlush => "final_flush",
        }
    }
}

/// Which audio source a job originated from. The scheduler uses this for
/// round-robin fairness at `Live` priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobSource {
    Mic,
    System,
}

/// Parameters for a single transcription request.
pub struct JobRequest {
    pub origin: JobOrigin,
    pub source: JobSource,
    pub wav_path: PathBuf,
    pub language: Option<String>,
    pub initial_prompt: Option<String>,
    pub diarization: bool,
}

/// Error from a scheduled job.
#[derive(Debug)]
pub enum SchedulerError {
    Cancelled,
    SidecarDead,
    NotInitialized,
    Transcription(String),
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerError::Cancelled => write!(f, "scheduler shut down before job completed"),
            SchedulerError::SidecarDead => write!(f, "sidecar died and could not be restarted"),
            SchedulerError::NotInitialized => write!(f, "transcription engine not initialized"),
            SchedulerError::Transcription(msg) => write!(f, "transcription failed: {msg}"),
        }
    }
}

impl std::error::Error for SchedulerError {}

/// Result of a completed job.
pub struct JobOutcome {
    pub result: Result<yapstack_transcription::TranscriptionResult, SchedulerError>,
    /// Wall-time the worker spent on this job (round-trip `transcribe_with`
    /// duration). Returned to callers so the existing per-chunk pressure
    /// telemetry keeps its `wall_ms` field honest under the scheduler.
    pub wall_ms: u64,
    /// Monotonic sequence assigned when the worker picks up the job. Used by
    /// the live-transcription loop as the `event_sequence` field on
    /// `LiveSegmentEvent` for stable frontend ordering.
    pub event_sequence: u64,
}

struct JobEnvelope {
    request: JobRequest,
    respond: oneshot::Sender<JobOutcome>,
}

struct SchedulerQueues {
    final_flush: VecDeque<JobEnvelope>,
    live_mic: VecDeque<JobEnvelope>,
    live_system: VecDeque<JobEnvelope>,
    backfill: VecDeque<JobEnvelope>,
    /// Last source served at `Live` priority — used for round-robin fairness.
    last_live_source: Option<JobSource>,
}

impl SchedulerQueues {
    fn new() -> Self {
        Self {
            final_flush: VecDeque::new(),
            live_mic: VecDeque::new(),
            live_system: VecDeque::new(),
            backfill: VecDeque::new(),
            last_live_source: None,
        }
    }

    /// Pick the next job in priority order, applying mic/system round-robin
    /// at the `Live` tier.
    fn pick_next(&mut self) -> Option<JobEnvelope> {
        if let Some(job) = self.final_flush.pop_front() {
            return Some(job);
        }
        let try_order = match self.last_live_source {
            Some(JobSource::Mic) => [JobSource::System, JobSource::Mic],
            _ => [JobSource::Mic, JobSource::System],
        };
        for src in try_order {
            let q = match src {
                JobSource::Mic => &mut self.live_mic,
                JobSource::System => &mut self.live_system,
            };
            if let Some(job) = q.pop_front() {
                self.last_live_source = Some(src);
                return Some(job);
            }
        }
        if let Some(job) = self.backfill.pop_front() {
            return Some(job);
        }
        None
    }

    fn push(&mut self, env: JobEnvelope) {
        match (env.request.origin, env.request.source) {
            (JobOrigin::FinalFlush, _) => self.final_flush.push_back(env),
            (JobOrigin::Live, JobSource::Mic) => self.live_mic.push_back(env),
            (JobOrigin::Live, JobSource::System) => self.live_system.push_back(env),
            (JobOrigin::Backfill, _) => self.backfill.push_back(env),
        }
    }

    /// Drain all pending jobs and send `Cancelled` responses.
    fn cancel_all(&mut self) {
        let drain = |q: &mut VecDeque<JobEnvelope>| {
            for env in q.drain(..) {
                let _ = env.respond.send(JobOutcome {
                    result: Err(SchedulerError::Cancelled),
                    wall_ms: 0,
                    event_sequence: 0,
                });
            }
        };
        drain(&mut self.final_flush);
        drain(&mut self.live_mic);
        drain(&mut self.live_system);
        drain(&mut self.backfill);
    }
}

struct SchedulerInner {
    queues: StdMutex<SchedulerQueues>,
    notify: Notify,
    /// The exclusive client held by the scheduler worker. `None` only during
    /// transient respawn windows. The worker is the sole caller of
    /// `transcribe_with`, so Arc uniqueness is guaranteed at `respawn` time.
    client: TokioMutex<Option<Arc<yapstack_transcription::TranscriptionClient>>>,
    /// Shared state we return the client to when the scheduler shuts down.
    shared_state: TranscriptionClientState,
    shutdown: AtomicBool,
    emit_seq: AtomicU64,
}

/// Priority-queue scheduler in front of a single sidecar lane.
pub struct TranscriptionScheduler {
    inner: Arc<SchedulerInner>,
    worker: TokioMutex<Option<JoinHandle<()>>>,
}

/// Default upper bound on how long `shutdown_and_return` waits for the worker
/// to drain remaining jobs. Large enough to absorb a 5-minute backfill window
/// transcribing on a slow sidecar; bounded so a wedged sidecar can't hang the
/// stop path forever. The worker exits as soon as the queue is empty, so the
/// typical drain finishes well under this cap.
pub const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 300;

impl TranscriptionScheduler {
    /// Create a new scheduler that owns `client` until `shutdown_and_return`
    /// is called. `shared_state` is the `TranscriptionClientState` the client
    /// was extracted from — the scheduler hands the client back to that
    /// state on shutdown so the next session/dictation finds it.
    pub fn new(
        client: Arc<yapstack_transcription::TranscriptionClient>,
        shared_state: TranscriptionClientState,
    ) -> Arc<Self> {
        let inner = Arc::new(SchedulerInner {
            queues: StdMutex::new(SchedulerQueues::new()),
            notify: Notify::new(),
            client: TokioMutex::new(Some(client)),
            shared_state,
            shutdown: AtomicBool::new(false),
            emit_seq: AtomicU64::new(0),
        });

        let worker_inner = inner.clone();
        let worker = tokio::spawn(async move {
            scheduler_worker(worker_inner).await;
        });

        Arc::new(Self {
            inner,
            worker: TokioMutex::new(Some(worker)),
        })
    }

    /// Submit a job to the scheduler. Returns a receiver that resolves when
    /// the worker completes the job (or fails / is cancelled on shutdown).
    pub fn submit(&self, request: JobRequest) -> oneshot::Receiver<JobOutcome> {
        let (tx, rx) = oneshot::channel();
        let env = JobEnvelope {
            request,
            respond: tx,
        };
        let mut queues = self.inner.queues.lock().expect("queue mutex poisoned");
        queues.push(env);
        drop(queues);
        self.inner.notify.notify_one();
        rx
    }

    /// Shut down the worker, drain remaining jobs (FinalFlush + Live +
    /// Backfill in priority order), and return the client to the shared
    /// state it was extracted from. Safe to call multiple times.
    ///
    /// `timeout` caps how long we wait for the worker to drain — beyond it,
    /// any unfinished jobs are cancelled and the client is taken back
    /// forcibly. Use `DEFAULT_SHUTDOWN_TIMEOUT_SECS` unless you have a
    /// reason to differ.
    pub async fn shutdown_and_return(&self, timeout: Duration) {
        if self.inner.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }
        // Wake the worker so it notices the shutdown flag.
        self.inner.notify.notify_waiters();

        let worker = {
            let mut guard = self.worker.lock().await;
            guard.take()
        };
        if let Some(worker) = worker {
            match tokio::time::timeout(timeout, worker).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!("scheduler worker joined with error: {}", e),
                Err(_) => warn!(
                    "scheduler worker did not exit within {}s — cancelling pending jobs",
                    timeout.as_secs()
                ),
            }
        }

        // Cancel any jobs the worker didn't get to (only non-empty if the
        // shutdown timeout fired before drain completed).
        {
            let mut queues = self.inner.queues.lock().expect("queue mutex poisoned");
            queues.cancel_all();
        }

        // Return the client to shared state. We hold the *only* Arc clone
        // outside the worker's transient in-call clones, so the Arc that
        // remains here can be put straight back; no try_unwrap needed since
        // shared_state already stores `Arc<TranscriptionClient>`.
        let mut private_guard = self.inner.client.lock().await;
        if let Some(arc_client) = private_guard.take() {
            let mut shared = self.inner.shared_state.lock().await;
            *shared = Some(arc_client);
            debug!("returned transcription client to shared state");
        }
    }
}

async fn scheduler_worker(inner: Arc<SchedulerInner>) {
    loop {
        let env = {
            let mut queues = inner.queues.lock().expect("queue mutex poisoned");
            queues.pick_next()
        };

        let Some(env) = env else {
            if inner.shutdown.load(Ordering::Acquire) {
                return;
            }
            inner.notify.notified().await;
            continue;
        };

        let event_sequence = inner.emit_seq.fetch_add(1, Ordering::Relaxed);
        let outcome = process_job(&inner, env.request, event_sequence).await;
        let _ = env.respond.send(outcome);
    }
}

async fn process_job(
    inner: &Arc<SchedulerInner>,
    request: JobRequest,
    event_sequence: u64,
) -> JobOutcome {
    let client_arc = {
        let guard = inner.client.lock().await;
        guard.as_ref().cloned()
    };
    let client = match client_arc {
        Some(c) => c,
        None => {
            return JobOutcome {
                result: Err(SchedulerError::NotInitialized),
                wall_ms: 0,
                event_sequence,
            };
        }
    };

    let wall_start = std::time::Instant::now();
    let result = client
        .transcribe_with(
            &request.wav_path,
            request.language.as_deref(),
            request.initial_prompt.as_deref(),
            request.diarization,
        )
        .await;
    let wall_ms = wall_start.elapsed().as_millis() as u64;

    match result {
        Ok(r) => JobOutcome {
            result: Ok(r),
            wall_ms,
            event_sequence,
        },
        Err(e) => {
            warn!(
                "scheduler: transcribe failed ({:?}/{:?}): {}",
                request.origin, request.source, e
            );
            // Drop our local Arc clone before attempting respawn so the
            // worker's Arc inside `inner.client` is the only remaining
            // reference. Otherwise `Arc::try_unwrap` fails.
            drop(client);

            let sidecar_dead = !client_is_running(inner).await;
            if sidecar_dead {
                info!("scheduler: sidecar died — attempting single respawn");
                if respawn_client(inner).await {
                    let client_arc = {
                        let guard = inner.client.lock().await;
                        guard.as_ref().cloned()
                    };
                    if let Some(client) = client_arc {
                        let retry_start = std::time::Instant::now();
                        let retry = client
                            .transcribe_with(
                                &request.wav_path,
                                request.language.as_deref(),
                                request.initial_prompt.as_deref(),
                                request.diarization,
                            )
                            .await;
                        return JobOutcome {
                            result: retry.map_err(|e| SchedulerError::Transcription(e.to_string())),
                            wall_ms: wall_ms + retry_start.elapsed().as_millis() as u64,
                            event_sequence,
                        };
                    }
                }
                return JobOutcome {
                    result: Err(SchedulerError::SidecarDead),
                    wall_ms,
                    event_sequence,
                };
            }

            JobOutcome {
                result: Err(SchedulerError::Transcription(e.to_string())),
                wall_ms,
                event_sequence,
            }
        }
    }
}

async fn client_is_running(inner: &Arc<SchedulerInner>) -> bool {
    let guard = inner.client.lock().await;
    match guard.as_ref() {
        Some(c) => c.is_running(),
        None => false,
    }
}

/// Try to respawn the sidecar. Returns `true` on success. The worker is the
/// only holder of the `Arc<TranscriptionClient>` at this point, so
/// `Arc::try_unwrap` always succeeds.
async fn respawn_client(inner: &Arc<SchedulerInner>) -> bool {
    let mut guard = inner.client.lock().await;
    let Some(arc_client) = guard.take() else {
        return false;
    };
    match Arc::try_unwrap(arc_client) {
        Ok(mut client) => match client.respawn().await {
            Ok(()) => {
                *guard = Some(Arc::new(client));
                info!("scheduler: sidecar respawned successfully");
                true
            }
            Err(e) => {
                error!("scheduler: sidecar respawn failed: {}", e);
                *guard = Some(Arc::new(client));
                false
            }
        },
        Err(still_shared) => {
            error!("scheduler: respawn skipped — Arc still shared (bug, please report)");
            *guard = Some(still_shared);
            false
        }
    }
}

/// Convert from the `AudioSourceLabel` DTO used elsewhere in
/// `live_transcription.rs` into the scheduler's internal `JobSource`.
pub fn source_from_label(label: super::live_transcription::AudioSourceLabel) -> JobSource {
    use super::live_transcription::AudioSourceLabel;
    match label {
        AudioSourceLabel::Mic => JobSource::Mic,
        AudioSourceLabel::System => JobSource::System,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_env(
        origin: JobOrigin,
        source: JobSource,
    ) -> (JobEnvelope, oneshot::Receiver<JobOutcome>) {
        let (tx, rx) = oneshot::channel();
        let env = JobEnvelope {
            request: JobRequest {
                origin,
                source,
                wav_path: PathBuf::from("/tmp/placeholder.wav"),
                language: None,
                initial_prompt: None,
                diarization: false,
            },
            respond: tx,
        };
        (env, rx)
    }

    #[test]
    fn priority_final_before_live_before_backfill() {
        let mut q = SchedulerQueues::new();
        let (backfill, _rx_b) = fake_env(JobOrigin::Backfill, JobSource::Mic);
        let (live, _rx_l) = fake_env(JobOrigin::Live, JobSource::Mic);
        let (final_flush, _rx_f) = fake_env(JobOrigin::FinalFlush, JobSource::Mic);
        q.push(backfill);
        q.push(live);
        q.push(final_flush);

        let first = q.pick_next().unwrap();
        assert!(matches!(first.request.origin, JobOrigin::FinalFlush));
        let second = q.pick_next().unwrap();
        assert!(matches!(second.request.origin, JobOrigin::Live));
        let third = q.pick_next().unwrap();
        assert!(matches!(third.request.origin, JobOrigin::Backfill));
        assert!(q.pick_next().is_none());
    }

    #[test]
    fn live_round_robin_alternates_sources() {
        let mut q = SchedulerQueues::new();
        for _ in 0..3 {
            let (m, _) = fake_env(JobOrigin::Live, JobSource::Mic);
            let (s, _) = fake_env(JobOrigin::Live, JobSource::System);
            q.push(m);
            q.push(s);
        }

        let mut sequence = Vec::new();
        while let Some(env) = q.pick_next() {
            sequence.push(env.request.source);
        }
        assert_eq!(sequence.len(), 6);
        assert_eq!(
            sequence,
            vec![
                JobSource::Mic,
                JobSource::System,
                JobSource::Mic,
                JobSource::System,
                JobSource::Mic,
                JobSource::System,
            ]
        );
    }

    #[test]
    fn live_drains_remaining_when_one_side_empty() {
        let mut q = SchedulerQueues::new();
        for _ in 0..3 {
            let (s, _) = fake_env(JobOrigin::Live, JobSource::System);
            q.push(s);
        }
        let mut count = 0;
        while let Some(env) = q.pick_next() {
            assert_eq!(env.request.source, JobSource::System);
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn final_flush_preempts_even_when_live_queued() {
        let mut q = SchedulerQueues::new();
        for _ in 0..2 {
            let (m, _) = fake_env(JobOrigin::Live, JobSource::Mic);
            q.push(m);
        }
        let (f, _) = fake_env(JobOrigin::FinalFlush, JobSource::System);
        q.push(f);
        let first = q.pick_next().unwrap();
        assert!(matches!(first.request.origin, JobOrigin::FinalFlush));
    }

    #[test]
    fn backfill_drains_after_live_even_when_interleaved() {
        // Regression guard: a backfill chunk that arrives before a live chunk
        // must still be served *after* the live chunk, not in submit order.
        let mut q = SchedulerQueues::new();
        let (b1, _) = fake_env(JobOrigin::Backfill, JobSource::Mic);
        q.push(b1);
        let (l1, _) = fake_env(JobOrigin::Live, JobSource::Mic);
        q.push(l1);
        let (b2, _) = fake_env(JobOrigin::Backfill, JobSource::Mic);
        q.push(b2);

        assert!(matches!(
            q.pick_next().unwrap().request.origin,
            JobOrigin::Live
        ));
        assert!(matches!(
            q.pick_next().unwrap().request.origin,
            JobOrigin::Backfill
        ));
        assert!(matches!(
            q.pick_next().unwrap().request.origin,
            JobOrigin::Backfill
        ));
    }

    #[test]
    fn cancel_all_drains_every_bucket() {
        let mut q = SchedulerQueues::new();
        let (final_flush, mut rx_f) = fake_env(JobOrigin::FinalFlush, JobSource::Mic);
        let (live, mut rx_l) = fake_env(JobOrigin::Live, JobSource::System);
        let (backfill, mut rx_b) = fake_env(JobOrigin::Backfill, JobSource::Mic);
        q.push(final_flush);
        q.push(live);
        q.push(backfill);

        q.cancel_all();

        for rx in [&mut rx_f, &mut rx_l, &mut rx_b] {
            let out = rx.try_recv().expect("waiter should have a value");
            assert!(matches!(out.result, Err(SchedulerError::Cancelled)));
        }
    }
}
