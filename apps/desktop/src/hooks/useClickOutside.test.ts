import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { useClickOutside } from "./useClickOutside";

describe("useClickOutside", () => {
  let callback: () => void;

  beforeEach(() => {
    callback = vi.fn();
  });

  it("calls callback when clicking outside the ref element", () => {
    const { result } = renderHook(() => useClickOutside(callback));
    const div = document.createElement("div");
    document.body.appendChild(div);
    result.current.current = div;

    const outsideEvent = new MouseEvent("mousedown", { bubbles: true });
    document.dispatchEvent(outsideEvent);

    expect(callback).toHaveBeenCalledTimes(1);
    document.body.removeChild(div);
  });

  it("does not call callback when clicking inside the ref element", () => {
    const { result } = renderHook(() => useClickOutside(callback));
    const div = document.createElement("div");
    const child = document.createElement("span");
    div.appendChild(child);
    document.body.appendChild(div);
    result.current.current = div;

    const insideEvent = new MouseEvent("mousedown", { bubbles: true });
    child.dispatchEvent(insideEvent);

    expect(callback).not.toHaveBeenCalled();
    document.body.removeChild(div);
  });

  it("does not call callback when disabled", () => {
    const { result } = renderHook(() => useClickOutside(callback, false));
    const div = document.createElement("div");
    document.body.appendChild(div);
    result.current.current = div;

    const outsideEvent = new MouseEvent("mousedown", { bubbles: true });
    document.dispatchEvent(outsideEvent);

    expect(callback).not.toHaveBeenCalled();
    document.body.removeChild(div);
  });

  it("cleans up listener on unmount", () => {
    const removeSpy = vi.spyOn(document, "removeEventListener");
    const { unmount } = renderHook(() => useClickOutside(callback));
    unmount();
    expect(removeSpy).toHaveBeenCalledWith("mousedown", expect.any(Function));
    removeSpy.mockRestore();
  });

  it("re-registers listener when enabled changes from false to true", () => {
    const div = document.createElement("div");
    document.body.appendChild(div);

    const { result, rerender } = renderHook(
      ({ enabled }) => useClickOutside(callback, enabled),
      { initialProps: { enabled: false } },
    );

    // Assign ref element
    result.current.current = div;

    // No listener initially (disabled)
    const outsideEvent = new MouseEvent("mousedown", { bubbles: true });
    document.dispatchEvent(outsideEvent);
    expect(callback).not.toHaveBeenCalled();

    // Enable — listener should be registered
    rerender({ enabled: true });
    document.dispatchEvent(outsideEvent);
    expect(callback).toHaveBeenCalledTimes(1);

    document.body.removeChild(div);
  });
});
