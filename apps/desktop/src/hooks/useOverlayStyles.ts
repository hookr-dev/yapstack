import { useEffect } from "react";

export function useOverlayStyles() {
  useEffect(() => {
    const style = document.createElement("style");
    style.textContent =
      "html, body, #root { background: transparent !important; background-color: transparent !important; }";
    document.head.appendChild(style);
    return () => style.remove();
  }, []);
}
