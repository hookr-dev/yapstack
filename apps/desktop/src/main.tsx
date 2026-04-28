import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Toaster } from "@/components/ui/sonner";
import "./index.css";

// Suppress the WebView's native right-click menu (Reload / Inspect Element
// in dev, Save Image As / Look Up / etc. in production) so the app feels
// like a native desktop app rather than a browser. React + Radix custom
// context menus are unaffected — they register their own component-level
// `onContextMenu` handlers that fire during the bubble phase before this
// window-level listener, and they call `preventDefault` themselves to open
// their own menu UI. `preventDefault` only suppresses the browser default
// action; it doesn't stop other listeners from firing, so the React event
// system continues to deliver context-menu events to components normally.
//
// Exception: real text inputs and contenteditable surfaces (Tiptap notes
// editor, dictation slot inputs, settings fields) keep the native menu so
// users still get system clipboard / paste / dictation / spell-check
// entries where they expect them. Everything else (cards, images, sidebar,
// chrome) is blocked.
//
// Bound on `window` rather than inside a React effect so the handler is
// installed exactly once per HMR cycle (the root reuse below preserves the
// React tree, but `main.tsx` itself only re-evaluates on a hard reload).
const NATIVE_MENU_ALLOWED_SELECTOR =
  'input, textarea, [contenteditable="true"], [contenteditable=""]';
window.addEventListener("contextmenu", (e) => {
  const target = e.target as Element | null;
  if (target?.closest?.(NATIVE_MENU_ALLOWED_SELECTOR)) return;
  e.preventDefault();
});

// On Vite HMR the root element already holds a mounted React tree. Calling
// createRoot a second time silently creates a second root and leaks the first
// (double listeners, duplicate event handlers, stale refs). Reuse the existing
// root across reloads by stashing it on the element itself.
const rootEl = document.getElementById("root")!;
type HmrRoot = ReactDOM.Root;
type RootElement = HTMLElement & { __yapstackRoot__?: HmrRoot };
const el = rootEl as RootElement;
const root = el.__yapstackRoot__ ?? ReactDOM.createRoot(rootEl);
el.__yapstackRoot__ = root;

root.render(
  <React.StrictMode>
    <App />
    <Toaster
      position="top-center"
      closeButton
      offset={46}
      duration={2500}
    />
  </React.StrictMode>,
);
