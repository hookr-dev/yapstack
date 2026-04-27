import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Toaster } from "@/components/ui/sonner";
import "./index.css";

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
