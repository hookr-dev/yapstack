import { useEffect, useRef } from "react";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { currentMonitor } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { register as registerGlobalShortcut, unregister as unregisterGlobalShortcut } from "@tauri-apps/plugin-global-shortcut";
import { useAppStore } from "@/stores/appStore";
import { commands } from "@/lib/tauri";
import { EVENTS, WINDOWS, listenEvent, emitEvent, type BubbleState } from "@/lib/events";
import { createAIClient, getActiveConfig, isAIConfigured } from "@/lib/ai";
import { buildVocabularyHints } from "@/lib/transcription";
import { createManualSession as dbCreateManualSession, saveNote, insertDictationHistory, listFolders, listTags } from "@/lib/db";
import { toast } from "sonner";
import { trackDictationStarted, trackDictationCompleted, trackDictationFailed, trackDictationCancelled } from "@/lib/analytics";

type DictationState = "idle" | "recording" | "transcribing" | "processing" | "cancelling" | "done";

// How long the bubble shows "Cancelled" before hiding. Mirrors the existing
// no-speech / error self-hide window.
const CANCEL_DISPLAY_MS = 450;
// How long to wait after stopLiveTranscription for the part to be finalized
// (session-part-ready) so we can delete it on cancel. Past this we give up.
const CANCEL_WAV_GRACE_MS = 1500;

const BUBBLE_WIDTH = 220;
const BUBBLE_HEIGHT = 96;
const BOTTOM_MARGIN = 30;

const ENERGY_SILENCE_THRESHOLD = 0.005;
const SILENT_POLLS_FOR_NO_INPUT = 6;
const ENERGY_POLL_MS = 500;
const ENERGY_WINDOW_SECS = 0.5;

const NON_ACTIONABLE_PATTERNS = new Set([
  "thank you", "thanks for watching", "thanks for listening",
  "bye", "the end", "subscribe", "like and subscribe",
  "please subscribe", "see you next time",
  "you", "yeah", "yes", "no", "okay", "oh", "hmm", "uh", "um", "so", "right",
  "...",
]);

function isNonActionable(text: string): boolean {
  const normalized = text.trim().toLowerCase().replace(/[.,!?;:]+$/, "");
  if (!normalized) return true;
  return NON_ACTIONABLE_PATTERNS.has(normalized);
}

async function showBubble() {
  try {
    const win = await WebviewWindow.getByLabel(WINDOWS.DICTATION);
    if (win) {
      const monitor = await currentMonitor();
      if (monitor) {
        const { position, size } = monitor;
        const scale = monitor.scaleFactor;
        const x = position.x + (size.width - BUBBLE_WIDTH * scale) / 2;
        const y =
          position.y + size.height - (BUBBLE_HEIGHT + BOTTOM_MARGIN) * scale;
        await win.setPosition(new PhysicalPosition(Math.round(x), Math.round(y)));
      }
    }
    await commands.showOverlayPanel(WINDOWS.DICTATION);
  } catch {
    // Bubble window may not exist yet
  }
}

async function hideBubble() {
  try {
    await commands.hideOverlayPanel(WINDOWS.DICTATION);
  } catch {
    // Graceful degradation
  }
}

async function emitBubbleState(state: BubbleState, slotName?: string) {
  await emitEvent(EVENTS.DICTATION_STATE, { state, slotName }).catch(() => {});
}

async function registerCancelHotkey() {
  try {
    await registerGlobalShortcut(["Escape"], (event) => {
      if (event.state === "Pressed") {
        window.dispatchEvent(new CustomEvent("yapstack:dictation-cancel"));
      }
    });
  } catch (e) {
    // Escape may already be registered (e.g. by another in-flight dictation)
    // or rejected by the OS; cancel via in-app focus still works.
    console.debug("Failed to register Escape cancel hotkey:", e);
  }
}

async function unregisterCancelHotkey() {
  try {
    await unregisterGlobalShortcut("Escape");
  } catch {
    // Not registered, or already unregistered — fine.
  }
}

async function focusMainWindow() {
  try {
    const win = await WebviewWindow.getByLabel(WINDOWS.MAIN);
    if (win) {
      await win.show();
      await win.setFocus();
    }
  } catch {
    // Graceful degradation
  }
}

/** Manages hold-to-talk dictation lifecycle: start, capture, transcribe, AI, output. Mounted in App.tsx. */
export function useDictation() {
  const stateRef = useRef<DictationState>("idle");
  const startTimeRef = useRef<number>(0);
  const slotIdRef = useRef<string>("");
  const dictationIdRef = useRef<string>("");
  const abortRef = useRef<AbortController | null>(null);
  const accumulatedTextRef = useRef<string>("");
  const unlistenSegmentRef = useRef<(() => void) | null>(null);
  const unlistenStatusRef = useRef<(() => void) | null>(null);
  const unlistenWavRef = useRef<(() => void) | null>(null);
  const stoppedDeferredRef = useRef<{ promise: Promise<void>; resolve: () => void } | null>(null);
  const wavInfoRef = useRef<{ path: string; duration: number } | null>(null);
  const energyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const silentPollCountRef = useRef(0);
  const noInputShownRef = useRef(false);

  useEffect(() => {
    // Typed read defeats TS control-flow narrowing of stateRef.current after
    // intermediate assignments (e.g. = "transcribing"), so cancel bail-points
    // can compare against "cancelling".
    const phase = (): DictationState => stateRef.current;

    function cleanupListeners() {
      unlistenSegmentRef.current?.();
      unlistenSegmentRef.current = null;
      unlistenStatusRef.current?.();
      unlistenStatusRef.current = null;
      unlistenWavRef.current?.();
      unlistenWavRef.current = null;
      stoppedDeferredRef.current = null;
      if (energyTimerRef.current) {
        clearTimeout(energyTimerRef.current);
        energyTimerRef.current = null;
      }
    }

    async function handleStart(e: Event) {
      const detail = (e as CustomEvent<{ slotId: string }>).detail;
      if (stateRef.current !== "idle") return;

      const s = useAppStore.getState();
      const { dictation } = s.settings;

      if (!dictation.enabled) {
        toast.error("Dictation is disabled. Enable it in Settings → Dictation.");
        return;
      }

      const slot = dictation.slots.find((sl) => sl.id === detail.slotId);
      if (!slot || !slot.enabled) {
        toast.error("This dictation slot is disabled.");
        return;
      }

      if (s.enginePhase !== "ready") {
        toast.error("Transcription engine is not ready.");
        return;
      }

      if (s.captureStatus?.state !== "Capturing") {
        toast.error("Audio capture is not active.");
        return;
      }

      // Set state before async work to prevent re-entry
      stateRef.current = "recording";
      startTimeRef.current = Date.now();
      slotIdRef.current = detail.slotId;
      dictationIdRef.current = crypto.randomUUID();
      accumulatedTextRef.current = "";
      wavInfoRef.current = null;

      // Register Escape as a Global hotkey for the duration of this Dictation
      // so the user can cancel from any app, not just YapStack's main window.
      registerCancelHotkey();
      // Tell the main store which synthetic session id dictation is using so
      // `onLiveSegment` can skip DB persistence for it (dictation session
      // isn't in the `sessions` table, and the FK would fail the insert).
      useAppStore.getState().setDictationSessionId(dictationIdRef.current);

      trackDictationStarted({
        slot_id: slot.id,
        slot_name: slot.name,
        ai_enabled: slot.aiEnabled ? 1 : 0,
        has_prompt: slot.prompt ? 1 : 0,
        output_action: slot.outputAction ?? "paste",
      });

      // Set up event listeners before starting live transcription
      const segmentUnlisten = await listenEvent(
        EVENTS.LIVE_TRANSCRIPTION_SEGMENT,
        (payload) => {
          for (const seg of payload.segments) {
            const text = seg.text.trim();
            if (text) {
              const isFirstText = !accumulatedTextRef.current.trim();
              accumulatedTextRef.current += (accumulatedTextRef.current ? " " : "") + text;
              // Always restore bubble on first text (belt-and-suspenders with energy polling)
              if (isFirstText && stateRef.current === "recording") {
                noInputShownRef.current = false;
                emitBubbleState("recording", slot.name);
              }
            }
          }
        },
      );
      unlistenSegmentRef.current = segmentUnlisten;

      // Listen for the part-ready event for this dictation. Dictation always
      // produces a single part per recording, so we just stash the file
      // path/duration for the dictation_history insert below.
      const currentDictId = dictationIdRef.current;
      const wavUnlisten = await listenEvent(
        EVENTS.SESSION_PART_READY,
        (payload) => {
          if (payload.session_id === currentDictId) {
            wavInfoRef.current = {
              path: payload.file_path,
              duration: payload.duration_seconds,
            };
          }
        },
      );
      unlistenWavRef.current = wavUnlisten;

      let resolveStop!: () => void;
      const stoppedPromise = new Promise<void>((r) => { resolveStop = r; });
      stoppedDeferredRef.current = { promise: stoppedPromise, resolve: resolveStop };

      const statusUnlisten = await listenEvent(
        EVENTS.LIVE_TRANSCRIPTION_STATUS,
        (payload) => {
          if (payload.phase === "Stopped" || payload.phase === "Error") {
            stoppedDeferredRef.current?.resolve();
          }
        },
      );
      unlistenStatusRef.current = statusUnlisten;

      // Start live transcription (pass dictation ID for WAV saving)
      const { language } = s.settings;
      const result = await commands.startLiveTranscription({
        silence_duration_ms: 400,
        max_chunk_seconds: 10,
        backfill_seconds: 0,
        source: "MicOnly",
        mix_config: null,
        language: language,
        prompt_context_chars: 350,
        prompt_decay_silence_seconds: 0,
        session_id: dictationIdRef.current,
        audio_save_location: null,
        audio_export_format: s.settings.audioExportFormat,
        mp3_bitrate: s.settings.audioExportFormat === "mp3" ? s.settings.mp3Bitrate : null,
        diarization:
          s.settings.selectedEngine === "Parakeet" && s.settings.diarizationEnabled,
        vocabulary_hints: buildVocabularyHints(await listFolders(), await listTags()),
      });

      if (result.status === "error") {
        cleanupListeners();
        setIdle();
        toast.error(`Dictation failed: ${result.error.message}`);
        trackDictationFailed({
          slot_id: slotIdRef.current,
          error_reason: result.error.message,
        });
        return;
      }

      // Guard: if handleStop or handleCancel fired while startLiveTranscription was
      // in-flight, stateRef will no longer be "recording" — tear down the ghost
      // transcription. Cancel owns its own cleanup; only run cleanup here when
      // we're in the handleStop ghost case.
      if (stateRef.current !== "recording") {
        await commands.stopLiveTranscription().catch(() => {});
        if (stateRef.current !== "cancelling") cleanupListeners();
        return;
      }

      emitBubbleState("recording", slot.name);
      showBubble();

      // Start energy-based no-input detection (setTimeout chain to avoid overlap)
      silentPollCountRef.current = 0;
      noInputShownRef.current = false;
      const slotName = slot.name;
      async function pollEnergy() {
        if (stateRef.current !== "recording") return;
        try {
          const result = await commands.peekCaptureEnergy(ENERGY_WINDOW_SECS);
          if (result.status === "error") return;
          const energy = result.data;
          const hasEnergy = (energy.mic_rms ?? 0) > ENERGY_SILENCE_THRESHOLD
                         || (energy.system_rms ?? 0) > ENERGY_SILENCE_THRESHOLD;
          if (hasEnergy) {
            silentPollCountRef.current = 0;
            if (noInputShownRef.current) {
              noInputShownRef.current = false;
              emitBubbleState("recording", slotName);
            }
          } else {
            silentPollCountRef.current++;
            if (silentPollCountRef.current >= SILENT_POLLS_FOR_NO_INPUT && !accumulatedTextRef.current.trim() && !noInputShownRef.current) {
              noInputShownRef.current = true;
              emitBubbleState("no-input", slotName);
            }
          }
        } catch (err) {
          if (stateRef.current === "recording") {
            console.debug("Energy poll failed:", err);
          }
        }
        if (stateRef.current === "recording") {
          energyTimerRef.current = setTimeout(pollEnergy, ENERGY_POLL_MS);
        }
      }
      energyTimerRef.current = setTimeout(pollEnergy, ENERGY_POLL_MS);
    }

    async function handleStop() {
      if (stateRef.current !== "recording") return;

      const s = useAppStore.getState();
      const slot = s.settings.dictation.slots.find((sl) => sl.id === slotIdRef.current);
      if (!slot) {
        setIdle();
        cleanupListeners();
        hideBubble();
        return;
      }

      const abort = new AbortController();
      abortRef.current = abort;

      try {
        // Signal stop — the loop will force-transcribe remaining audio
        stateRef.current = "transcribing";
        emitBubbleState("transcribing", slot.name);

        await commands.stopLiveTranscription();
        if (phase() === "cancelling") return;

        // Wait for "Stopped" event (final chunks will have been emitted)
        if (stoppedDeferredRef.current) {
          const timeout = new Promise<void>((resolve) => setTimeout(resolve, 5000));
          await Promise.race([stoppedDeferredRef.current.promise, timeout]);
        }
        if (phase() === "cancelling") return;

        let text = accumulatedTextRef.current.trim();
        const inputText = text; // Capture raw transcription before AI

        if (isNonActionable(text)) {
          stateRef.current = "done";
          emitBubbleState("no-speech");
          setTimeout(() => {
            hideBubble();
            setIdle();
          }, 1200);
          return;
        }

        // AI processing
        if (slot.aiEnabled && slot.prompt) {
          stateRef.current = "processing";
          emitBubbleState("processing", slot.name);

          const aiSettings = s.settings.ai;
          const config = getActiveConfig(aiSettings);

          if (!isAIConfigured(aiSettings)) {
            // AI not configured for this provider — fall through to raw transcription.
            // For custom providers a blank apiKey is valid (local servers), so we
            // check both baseUrl + model presence via isAIConfigured.
          } else {
            try {
              const client = createAIClient(aiSettings);
              const response = await client.chat.completions.create(
                {
                  model: config.model,
                  messages: [
                    { role: "system", content: slot.prompt },
                    { role: "user", content: text },
                  ],
                },
                { signal: abort.signal },
              );
              const processed = response.choices[0]?.message?.content;
              if (processed) {
                text = processed.trim();
              }
            } catch (aiErr) {
              if (abort.signal.aborted) return;
              console.error("AI processing failed, using raw text:", aiErr);
            }
          }
        }

        if (phase() === "cancelling") return;

        // Output based on slot action
        const action = slot.outputAction ?? "paste";

        let resultState: BubbleState;
        let noteSessionId: string | null = null;

        if (action === "new-note") {
          try {
            noteSessionId = crypto.randomUUID();
            const sessionId = noteSessionId;
            const title = text.slice(0, 60);
            await dbCreateManualSession(sessionId, title);
            await saveNote(sessionId, `<p>${text}</p>`);
            await focusMainWindow();
            const store = useAppStore.getState();
            await store.loadSessions();
            await store.openSession(sessionId);
            resultState = "note-created";
          } catch (noteErr) {
            console.error("Failed to create note from dictation:", noteErr);
            await commands.clipboardPaste(text, false);
            resultState = "copied";
          }
        } else {
          const shouldPaste = action === "paste";
          if (shouldPaste) {
            // Hide bubble first so focus returns to the target app
            await hideBubble();
            await new Promise((r) => setTimeout(r, 100));
          }
          const pasteResult = await commands.clipboardPaste(text, shouldPaste);
          if (pasteResult.status === "error") {
            console.error("clipboard_paste error:", pasteResult.error.message);
            resultState = "error";
          } else {
            resultState = shouldPaste ? "pasted" : "copied";
          }
        }

        if (phase() === "cancelling") return;

        trackDictationCompleted({
          slot_id: slotIdRef.current,
          duration_ms: Date.now() - startTimeRef.current,
          transcription_length: text.length,
          ai_processed: slot.aiEnabled && slot.prompt ? 1 : 0,
          output_action: action,
        });

        // Allow WAV finalization
        if (!wavInfoRef.current) {
          await new Promise((r) => setTimeout(r, 500));
        }

        if (phase() === "cancelling") return;

        // Persist dictation history (awaited so refresh sees the new entry)
        try {
          await insertDictationHistory({
            id: dictationIdRef.current,
            slot_id: slot.id,
            slot_name: slot.name,
            input_text: inputText,
            output_text: text,
            ai_enabled: slot.aiEnabled && slot.prompt ? 1 : 0,
            ai_prompt: slot.aiEnabled ? slot.prompt : null,
            output_action: action,
            wav_file_path: wavInfoRef.current?.path ?? null,
            wav_duration_seconds: wavInfoRef.current?.duration ?? null,
            session_id: noteSessionId,
          });
        } catch (e) {
          console.error("Failed to save dictation history:", e);
        }

        stateRef.current = "done";
        if (resultState === "error") {
          // Surface only failures to the user — success is implied by the
          // paste/copy/note action itself, so don't add an extra "Done" popup.
          emitBubbleState("error");
          await showBubble();
          setTimeout(() => {
            hideBubble();
            setIdle();
          }, 1200);
        } else {
          // Success: hide immediately. Paste path already hid the bubble
          // pre-paste for focus transfer; copy/note paths hide it here.
          await hideBubble();
          setIdle();
        }
      } catch (err) {
        if (abort.signal.aborted) return;
        console.error("Dictation failed:", err);
        trackDictationFailed({
          slot_id: slotIdRef.current,
          error_reason: err instanceof Error ? err.message : String(err),
        });
        stateRef.current = "done";
        emitBubbleState("error");
        setTimeout(() => {
          hideBubble();
          setIdle();
        }, 1200);
      } finally {
        cleanupListeners();
        abortRef.current = null;
      }
    }

    function setIdle() {
      stateRef.current = "idle";
      useAppStore.getState().setDictationSessionId(null);
      unregisterCancelHotkey();
      // Notify toggle mode that dictation is done (clears toggle state)
      window.dispatchEvent(new CustomEvent("yapstack:dictation-idle"));
    }

    async function handleCancel() {
      const startPhase = phase();

      // Nothing to cancel.
      if (startPhase === "idle" || startPhase === "cancelling") return;

      // Post-success / post-error display window: just dismiss the bubble fast.
      // No teardown work remains.
      if (startPhase === "done") {
        await hideBubble();
        setIdle();
        return;
      }

      // Take ownership of the lifecycle. Any in-flight handleStart guard or
      // handleStop bail-point will see "cancelling" and exit without running
      // its own teardown.
      stateRef.current = "cancelling";

      abortRef.current?.abort();
      // Unblock handleStop's wait on the Stopped/Error status event.
      stoppedDeferredRef.current?.resolve();

      emitBubbleState("cancelled");

      trackDictationCancelled({
        slot_id: slotIdRef.current,
        phase: startPhase,
        duration_ms: Date.now() - startTimeRef.current,
      });

      // Stop live transcription — finalizes the session part (WAV or MP3 per
      // audioExportFormat) and emits "Stopped". Capture itself is app-wide
      // and stays running.
      await commands.stopLiveTranscription().catch(() => {});

      // Wait briefly for the part to finalize so we can delete it.
      // wavInfoRef populates from the session-part-ready listener, which is
      // still mounted at this point.
      const wavDeadline = Date.now() + CANCEL_WAV_GRACE_MS;
      while (!wavInfoRef.current && Date.now() < wavDeadline) {
        await new Promise((r) => setTimeout(r, 100));
      }

      if (wavInfoRef.current) {
        await commands.deleteSessionWav(dictationIdRef.current, null).catch(() => {});
      }

      cleanupListeners();
      abortRef.current = null;

      // Hold the "Cancelled" bubble briefly for visual confirmation, then hide.
      await new Promise((r) => setTimeout(r, CANCEL_DISPLAY_MS));
      await hideBubble();
      setIdle();
    }

    window.addEventListener("yapstack:dictation-start", handleStart);
    window.addEventListener("yapstack:dictation-stop", handleStop);
    window.addEventListener("yapstack:dictation-cancel", handleCancel);

    return () => {
      window.removeEventListener("yapstack:dictation-start", handleStart);
      window.removeEventListener("yapstack:dictation-stop", handleStop);
      window.removeEventListener("yapstack:dictation-cancel", handleCancel);
      abortRef.current?.abort();
      // Stop live transcription if dictation is active on teardown
      if (stateRef.current !== "idle" && stateRef.current !== "done") {
        commands.stopLiveTranscription().catch(() => {});
        unregisterCancelHotkey();
      }
      cleanupListeners();
    };
  }, []);
}
