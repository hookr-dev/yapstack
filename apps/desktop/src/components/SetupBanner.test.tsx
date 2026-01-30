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
import { useAppStore } from "@/stores/appStore";
import { SetupBanner } from "./SetupBanner";

vi.mock("@tauri-apps/api/core", () => tauriCoreMock());
vi.mock("@tauri-apps/api/event", () => tauriEventMock());
vi.mock("@tauri-apps/api/window", () => tauriWindowMock());
vi.mock("@tauri-apps/api/dpi", () => tauriDpiMock());
vi.mock("@tauri-apps/api/webviewWindow", () => tauriWebviewWindowMock());
vi.mock("@tauri-apps/plugin-sql", () => tauriSqlMock());
vi.mock("@/lib/tauri", () => tauriCommandsMock());

beforeEach(() => {
  vi.clearAllMocks();
});

describe("SetupBanner", () => {
  it("renders nothing when engine is ready", () => {
    useAppStore.setState({ enginePhase: "ready" });
    const { container } = render(<SetupBanner />);
    expect(container.firstChild).toBeNull();
  });

  it("shows progress when downloading", () => {
    useAppStore.setState({
      enginePhase: "downloading",
      modelDownloadProgress: 50,
    });
    render(<SetupBanner />);
    expect(screen.getByText(/Downloading.*model/)).toBeInTheDocument();
  });

  it("shows loading message when initializing", () => {
    useAppStore.setState({ enginePhase: "initializing" });
    render(<SetupBanner />);
    expect(
      screen.getByText("Loading transcription engine..."),
    ).toBeInTheDocument();
  });

  it("renders nothing for idle phase", () => {
    useAppStore.setState({ enginePhase: "idle" });
    const { container } = render(<SetupBanner />);
    expect(container.firstChild).toBeNull();
  });
});
