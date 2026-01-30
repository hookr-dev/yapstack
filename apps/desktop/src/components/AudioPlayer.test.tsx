import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { AudioPlayer } from "./AudioPlayer";

// Mock HTMLAudioElement play/pause
beforeEach(() => {
  vi.clearAllMocks();
  // jsdom doesn't implement play/pause
  HTMLAudioElement.prototype.play = vi.fn().mockResolvedValue(undefined);
  HTMLAudioElement.prototype.pause = vi.fn();
});

describe("AudioPlayer", () => {
  it("renders play button and time displays", () => {
    render(<AudioPlayer src="test.wav" duration={120} />);
    // Duration displays
    expect(screen.getByText("2:00")).toBeInTheDocument();
    // Current time (0:00)
    expect(screen.getByText("0:00")).toBeInTheDocument();
    // Speed button
    expect(screen.getByText("1x")).toBeInTheDocument();
  });

  it("toggles to pause icon on play", async () => {
    render(<AudioPlayer src="test.wav" duration={60} />);
    // Find the play/pause button (ghost variant, icon-xs size)
    const buttons = screen.getAllByRole("button");
    const playBtn = buttons[0];
    await userEvent.click(playBtn);
    expect(HTMLAudioElement.prototype.play).toHaveBeenCalled();
  });

  it("cycles through playback speeds", async () => {
    render(<AudioPlayer src="test.wav" duration={60} />);
    const speedBtn = screen.getByText("1x");
    await userEvent.click(speedBtn);
    expect(screen.getByText("1.25x")).toBeInTheDocument();
    await userEvent.click(screen.getByText("1.25x"));
    expect(screen.getByText("1.5x")).toBeInTheDocument();
  });

  it("displays correct duration", () => {
    render(<AudioPlayer src="test.wav" duration={605} />);
    expect(screen.getByText("10:05")).toBeInTheDocument();
  });

  it("calls onPlayStateChange callback", async () => {
    const onPlayStateChange = vi.fn();
    render(
      <AudioPlayer
        src="test.wav"
        duration={60}
        onPlayStateChange={onPlayStateChange}
      />,
    );
    const buttons = screen.getAllByRole("button");
    await userEvent.click(buttons[0]);
    expect(onPlayStateChange).toHaveBeenCalledWith(true);
  });
});
