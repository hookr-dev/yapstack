import { useEffect, useRef, type RefObject } from "react";

/** Returns a ref — calls `onClickOutside` when a mousedown occurs outside the ref element. */
export function useClickOutside<T extends HTMLElement>(
  onClickOutside: () => void,
  enabled = true,
): RefObject<T | null> {
  const ref = useRef<T | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const handleMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onClickOutside();
      }
    };
    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, [onClickOutside, enabled]);

  return ref;
}
