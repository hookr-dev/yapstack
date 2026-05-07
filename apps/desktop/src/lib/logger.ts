/**
 * Frontend → backend log bridge.
 *
 * Forwards structured log events to the unified Rust `tracing` subscriber so
 * frontend issues land in the same places as native logs: stderr, the
 * rolling daily file under `app_log_dir()`, and the in-memory ring buffer
 * tailed by the LogsPanel. See `apps/desktop/src-tauri/src/logging.rs`.
 *
 * What the bridge captures:
 *   1. Explicit `log.error/warn/info/debug` calls from app code.
 *   2. Every `console.error` / `console.warn` (existing call sites flow
 *      through unchanged — they keep printing to devtools AND get forwarded).
 *   3. Uncaught errors (`window.onerror`).
 *   4. Unhandled promise rejections (`unhandledrejection`).
 *   5. Manual JS-heap snapshots via `captureDiagnostics`.
 *
 * Periodic memory sampling lives Rust-side
 * (`logging::spawn_memory_sampler`) and covers every platform. We don't
 * duplicate it here — `performance.memory` is unavailable on WKWebView and
 * the per-platform JS-heap delta we'd capture on Chromium webviews adds
 * little on top of process-level RSS.
 *
 * PII contract: same rule as the Rust subscriber — never log transcript
 * text. Console wrappers stringify whatever the caller passed; existing
 * call sites in this codebase already follow the rule.
 */

import { commands } from "@/lib/tauri";

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

let installed = false;

/** Stringify whatever the caller handed `console.error(...)`: `Error`s come
 * out with stack, plain values use a cycle-safe JSON dump. */
export function safeStringify(value: unknown): string {
  if (value instanceof Error) {
    return value.stack ?? `${value.name}: ${value.message}`;
  }
  if (typeof value === "string") return value;
  if (value === null || value === undefined) return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return Object.prototype.toString.call(value);
  }
}

export function formatArgs(args: readonly unknown[]): string {
  return args.map(safeStringify).join(" ");
}

/** Fire-and-forget send. Failures inside the logger must never throw or
 * recurse — the whole point is that something else is already broken
 * when these fire. */
function send(level: LogLevel, message: string, module?: string): void {
  void commands.logFrontend(level, module ?? null, message).catch(() => {
    /* never propagate */
  });
}

/** Direct logging surface for app code that wants to opt in explicitly. */
export const log = {
  error: (message: string, module?: string) => send("error", message, module),
  warn: (message: string, module?: string) => send("warn", message, module),
  info: (message: string, module?: string) => send("info", message, module),
  debug: (message: string, module?: string) => send("debug", message, module),
};

/** Wrap one console method to forward to the backend while preserving the
 * original devtools output. Closing over `original` keeps it private to
 * the wrapper — no module-level mutable state. */
function wrapConsole(level: "error" | "warn"): void {
  const original = console[level].bind(console);
  console[level] = (...args: unknown[]) => {
    original(...args);
    send(level, formatArgs(args), "console");
  };
}

/** WebKit / WebView2 expose this; WKWebView does not. Used only by the
 * manual snapshot path — periodic sampling is the Rust side's job. */
interface MemoryInfo {
  usedJSHeapSize: number;
  totalJSHeapSize: number;
  jsHeapSizeLimit: number;
}

function getMemory(): MemoryInfo | null {
  const perf = performance as Performance & { memory?: MemoryInfo };
  return perf.memory ?? null;
}

/** Install the bridge. Idempotent so HMR or accidental double-init can't
 * stack two console-wrappers. */
export function installFrontendLogger(): void {
  if (installed) return;
  installed = true;

  // Wrap only error/warn. Wrapping log/debug would forward hot-path
  // FE traces (e.g. the per-chunk `console.debug` in `appStore`) into
  // the backend at >10 Hz. The opt-in `log.debug(...)` API is the
  // surface for FE callers that want debug events archived.
  wrapConsole("error");
  wrapConsole("warn");

  // Uncaught synchronous errors. `event.error` is the Error object on
  // modern browsers; fall back to the message + location triplet otherwise.
  window.addEventListener("error", (event) => {
    const detail =
      event.error instanceof Error
        ? safeStringify(event.error)
        : event.message || "Unknown window error";
    const where =
      event.filename || event.lineno
        ? ` (at ${event.filename}:${event.lineno}:${event.colno})`
        : "";
    send("error", `${detail}${where}`, "window.error");
  });

  // Unhandled promise rejections (await without try, dangling .then chains).
  window.addEventListener("unhandledrejection", (event) => {
    send("error", safeStringify(event.reason), "unhandledrejection");
  });

  send("info", "frontend logger installed", "init");
}

/** Capture a one-shot diagnostic snapshot — heap stats + caller-supplied
 * counters. Routed through `info` so it lands in the rolling log. Surfaced
 * via the LogsPanel "Snapshot" button so the user can mark "right now is
 * interesting" without devtools. */
export function captureDiagnostics(extras?: Record<string, string | number>): void {
  const m = getMemory();
  const heap = m
    ? `usedMB=${(m.usedJSHeapSize / 1048576).toFixed(1)} totalMB=${(m.totalJSHeapSize / 1048576).toFixed(1)} limitMB=${(m.jsHeapSizeLimit / 1048576).toFixed(0)}`
    : "memory-api=unavailable";
  const tail = extras
    ? Object.entries(extras).map(([k, v]) => ` ${k}=${v}`).join("")
    : "";
  send("info", `${heap}${tail}`, "diagnostics");
}
