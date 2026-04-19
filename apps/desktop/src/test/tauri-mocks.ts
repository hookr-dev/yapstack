import { vi } from "vitest";

/**
 * Shared mock factory functions for Tauri APIs.
 *
 * IMPORTANT: vi.mock() calls must be in the test file itself for hoisting to work.
 * Import these factories and use them in vi.mock() calls:
 *
 *   import { tauriCoreMock, tauriEventMock, ... } from "@/test/tauri-mocks";
 *   vi.mock("@tauri-apps/api/core", () => tauriCoreMock());
 */

export const tauriCoreMock = () => ({
  invoke: vi.fn().mockResolvedValue(null),
});

export const tauriEventMock = () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
  emit: vi.fn(),
});

export const tauriWindowMock = () => ({
  getCurrentWindow: vi.fn().mockReturnValue({
    setPosition: vi.fn().mockResolvedValue(undefined),
    setSize: vi.fn().mockResolvedValue(undefined),
    outerPosition: vi.fn().mockResolvedValue({ x: 0, y: 0 }),
    innerSize: vi.fn().mockResolvedValue({ width: 1100, height: 750 }),
    onCloseRequested: vi.fn().mockResolvedValue(() => {}),
    onMoved: vi.fn().mockResolvedValue(() => {}),
    onResized: vi.fn().mockResolvedValue(() => {}),
    hide: vi.fn().mockResolvedValue(undefined),
    show: vi.fn().mockResolvedValue(undefined),
    setFocus: vi.fn().mockResolvedValue(undefined),
  }),
});

export const tauriDpiMock = () => ({
  PhysicalPosition: vi.fn(),
  PhysicalSize: vi.fn(),
});

export const tauriWebviewWindowMock = () => ({
  WebviewWindow: {
    getByLabel: vi.fn().mockResolvedValue(null),
  },
});

export const tauriSqlMock = () => ({
  default: {
    load: vi.fn().mockResolvedValue({
      select: vi.fn().mockResolvedValue([]),
      execute: vi.fn().mockResolvedValue(null),
    }),
  },
});

export const tauriDialogMock = () => ({
  open: vi.fn().mockResolvedValue(null),
  save: vi.fn().mockResolvedValue(null),
  message: vi.fn().mockResolvedValue(undefined),
  ask: vi.fn().mockResolvedValue(false),
  confirm: vi.fn().mockResolvedValue(false),
});

export const tauriFsMock = () => ({
  readTextFile: vi.fn().mockResolvedValue(""),
  writeTextFile: vi.fn().mockResolvedValue(undefined),
  exists: vi.fn().mockResolvedValue(false),
  mkdir: vi.fn().mockResolvedValue(undefined),
  remove: vi.fn().mockResolvedValue(undefined),
});

export const tauriOpenerMock = () => ({
  revealItemInDir: vi.fn().mockResolvedValue(undefined),
  openUrl: vi.fn().mockResolvedValue(undefined),
});

export const tauriGlobalShortcutMock = () => ({
  register: vi.fn().mockResolvedValue(undefined),
  unregister: vi.fn().mockResolvedValue(undefined),
  unregisterAll: vi.fn().mockResolvedValue(undefined),
  isRegistered: vi.fn().mockResolvedValue(false),
});

export const tauriPathMock = () => ({
  appDataDir: vi.fn().mockResolvedValue("/mock/app-data"),
  appConfigDir: vi.fn().mockResolvedValue("/mock/app-config"),
});

export const tauriCommandsMock = () => ({
  commands: {
    healthCheck: vi.fn().mockResolvedValue({ status: "ok", version: "0.1.0" }),
    listAudioDevices: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    getDefaultInputDevice: vi
      .fn()
      .mockResolvedValue({ status: "error", error: "no device" }),
    startCapture: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    stopCapture: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    getCaptureStatus: vi.fn().mockResolvedValue({
      status: "ok",
      data: {
        state: "Idle",
        mic_active: false,
        system_audio_active: false,
        error_message: null,
      },
    }),
    checkSystemAudioPermission: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: "Granted" }),
    snapshotMicAudio: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    snapshotSystemAudio: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    getBufferInfo: vi.fn().mockResolvedValue({
      status: "ok",
      data: { mic: null, system: null },
    }),
    triggerInstantCapture: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    startSession: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    endSession: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    getSessionStatus: vi.fn().mockResolvedValue({
      status: "ok",
      data: { active: false, elapsed_seconds: null },
    }),
    getAvailableModels: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: [] }),
    downloadModel: vi.fn().mockResolvedValue({ status: "ok", data: "" }),
    deleteModel: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    transcribeAudio: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    initWhisperClient: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    shutdownWhisperClient: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    getWhisperStatus: vi.fn().mockResolvedValue({
      status: "ok",
      data: { initialized: false },
    }),
    startLiveTranscription: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    stopLiveTranscription: vi
      .fn()
      .mockResolvedValue({ status: "ok", data: null }),
    getLiveTranscriptionStatus: vi.fn().mockResolvedValue({
      status: "ok",
      data: { phase: "Stopped" },
    }),
    clipboardPaste: vi.fn().mockResolvedValue({ status: "ok", data: null }),
  },
});
