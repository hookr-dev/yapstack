interface MarqueeRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface MarqueeOverlayProps {
  marquee: MarqueeRect | null;
}

export function MarqueeOverlay({ marquee }: MarqueeOverlayProps) {
  if (!marquee) return null;
  return (
    <div
      className="pointer-events-none absolute rounded border border-primary/50 bg-primary/10"
      style={{
        left: marquee.left,
        top: marquee.top,
        width: marquee.width,
        height: marquee.height,
      }}
    />
  );
}
