import "@testing-library/jest-dom/vitest";

// Polyfill ResizeObserver for react-resizable-panels in jsdom
(globalThis as Record<string, unknown>).ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};

// Polyfill crypto.randomUUID for jsdom
if (!globalThis.crypto?.randomUUID) {
  let counter = 0;
  Object.defineProperty(globalThis, "crypto", {
    value: {
      ...globalThis.crypto,
      randomUUID: () => `00000000-0000-0000-0000-${String(++counter).padStart(12, "0")}`,
    },
    writable: true,
  });
}
