import { useEffect } from "react";
import { toast } from "sonner";
import { useAppStore } from "@/stores/appStore";
import { commands } from "@/lib/tauri";
import { EVENTS, listenEvent } from "@/lib/events";
import { trackStreamHealthEvent } from "@/lib/analytics";

/** Listens for live transcription segment, backfill, and status events. Mounted in AppLayout. */
export function useLiveTranscriptionEvents() {
  const setLivePhase = useAppStore((s) => s.setLivePhase);
  const onLiveSegment = useAppStore((s) => s.onLiveSegment);
  const onBackfillComplete = useAppStore((s) => s.onBackfillComplete);
  const onSessionPartReady = useAppStore((s) => s.onSessionPartReady);
  const recoverActiveSession = useAppStore((s) => s.recoverActiveSession);

  useEffect(() => {
    const unlisten = Promise.all([
      listenEvent(EVENTS.LIVE_TRANSCRIPTION_SEGMENT, (payload) => {
        onLiveSegment(payload);
      }),
      listenEvent(EVENTS.LIVE_TRANSCRIPTION_STATUS, (payload) => {
        setLivePhase(payload.phase);

        // Show toast for Error phase
        if (payload.phase === "Error" && payload.error_message) {
          toast.error(payload.error_message);
        }
      }),
      listenEvent(EVENTS.TRANSCRIPTION_ENGINE_LOADED, (payload) => {
        // Source of truth for "what's actually running" — emitted right
        // after the sidecar's first model_loaded probe and on every
        // subsequent engine/variant switch. StatusPopover renders this.
        useAppStore.setState({
          loadedEngineInfo: {
            engine: payload.engine,
            accel: payload.accel,
            modelDir: payload.model_dir,
          },
        });
      }),
      listenEvent(EVENTS.LIVE_TRANSCRIPTION_WARNING, (payload) => {
        toast.warning(payload.message, { id: "transcription-warning" });
      }),
      listenEvent(EVENTS.BACKFILL_COMPLETE, () => {
        onBackfillComplete();
      }),
      listenEvent(EVENTS.SESSION_PART_READY, (payload) => {
        onSessionPartReady(payload);
      }),
      listenEvent(EVENTS.SESSION_WAV_ERROR, (payload) => {
        toast.error(payload.message);
      }),
      listenEvent(EVENTS.SESSION_WAV_WARNING, (payload) => {
        toast.warning(payload.message, { id: `wav-warning-${payload.session_id}` });
      }),
      listenEvent(EVENTS.STREAM_HEALTH, (payload) => {
        trackStreamHealthEvent({
          source: payload.source,
          status: payload.status,
        });
        const toastId = `stream-health-${payload.source}`;
        if (payload.status === "restarted") {
          toast.success(payload.message, { id: toastId, duration: 3000 });
        } else {
          toast.error(payload.message, {
            id: toastId,
            duration: payload.status === "restart_abandoned" ? Infinity : 5000,
          });
        }
      }),
    ]);

    // Eager fetch — recover state on mount/reload
    commands.getLiveTranscriptionStatus().then((r) => {
      if (r.status === "ok") {
        setLivePhase(r.data.phase);
        // If backend is still running with a session, recover frontend state
        if (r.data.phase === "Running" && r.data.session_id) {
          recoverActiveSession(r.data.session_id, r.data.effective_start_epoch_ms ?? undefined);
        }
      }
    });

    return () => {
      unlisten.then((fns) => fns.forEach((fn) => fn()));
    };
  }, [setLivePhase, onLiveSegment, onBackfillComplete, onSessionPartReady, recoverActiveSession]);
}
