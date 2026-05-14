import { forwardRef, useImperativeHandle, useRef } from "react";

export interface MarqueeRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface MarqueeOverlayHandle {
  setRect(rect: MarqueeRect): void;
  hide(): void;
}

// Imperative overlay: the rect is mutated directly on the DOM node so a
// drag at ~120 Hz doesn't push a setState through React on every move.
// The parent holds a ref to this handle and calls setRect/hide from
// pointermove and pointerup; nothing about the rect lives in React state.
export const MarqueeOverlay = forwardRef<MarqueeOverlayHandle>(
  function MarqueeOverlay(_, handleRef) {
    const divRef = useRef<HTMLDivElement | null>(null);

    useImperativeHandle(
      handleRef,
      () => ({
        setRect(rect) {
          const el = divRef.current;
          if (!el) return;
          el.style.left = `${rect.left}px`;
          el.style.top = `${rect.top}px`;
          el.style.width = `${rect.width}px`;
          el.style.height = `${rect.height}px`;
          el.style.display = "block";
        },
        hide() {
          const el = divRef.current;
          if (!el) return;
          el.style.display = "none";
        },
      }),
      [],
    );

    return (
      <div
        ref={divRef}
        className="pointer-events-none absolute rounded border border-primary/50 bg-primary/10"
        style={{ display: "none" }}
      />
    );
  },
);
