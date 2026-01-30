import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  tauriCoreMock,
  tauriEventMock,
  tauriWindowMock,
  tauriDpiMock,
  tauriWebviewWindowMock,
  tauriSqlMock,
  tauriCommandsMock,
} from "@/test/tauri-mocks";
import { setupMatchMedia } from "@/test/match-media";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => tauriCoreMock());
vi.mock("@tauri-apps/api/event", () => tauriEventMock());
vi.mock("@tauri-apps/api/window", () => tauriWindowMock());
vi.mock("@tauri-apps/api/dpi", () => tauriDpiMock());
vi.mock("@tauri-apps/api/webviewWindow", () => tauriWebviewWindowMock());
vi.mock("@tauri-apps/plugin-sql", () => tauriSqlMock());
vi.mock("@/lib/tauri", () => tauriCommandsMock());

beforeEach(() => {
  vi.clearAllMocks();
  setupMatchMedia();
});

describe("App", () => {
  it("renders the sidebar navigation", () => {
    render(<App />);
    expect(screen.getAllByText("All Sessions").length).toBeGreaterThan(0);
  });

  it("renders the settings button", () => {
    render(<App />);
    expect(screen.getByLabelText("Settings")).toBeInTheDocument();
  });

  it("shows waiting message when engine is not ready", () => {
    render(<App />);
    expect(
      screen.getByText(
        "Waiting for engine and audio capture to be ready...",
      ),
    ).toBeInTheDocument();
  });
});
