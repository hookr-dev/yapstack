import { useEffect, useState } from "react";
import { YapStackIcon } from "@/components/YapStackIcon";
import { EVENTS, listenEvent, type BubbleState } from "@/lib/events";
import { useOverlayStyles } from "@/hooks/useOverlayStyles";

const stateConfig: Record<
  BubbleState,
  { label: string; ringColor: string; textColor: string; glow: string; animate?: string }
> = {
  recording: {
    label: "Listening...",
    ringColor: "ring-red-500",
    textColor: "text-red-400",
    glow: "shadow-[0_0_12px_rgba(239,68,68,0.5)]",
    animate: "animate-pulse",
  },
  transcribing: {
    label: "Transcribing",
    ringColor: "ring-blue-500",
    textColor: "text-blue-400",
    glow: "shadow-[0_0_12px_rgba(59,130,246,0.5)]",
  },
  processing: {
    label: "Processing",
    ringColor: "ring-purple-500",
    textColor: "text-purple-400",
    glow: "shadow-[0_0_12px_rgba(168,85,247,0.5)]",
  },
  pasted: {
    label: "Pasted",
    ringColor: "ring-green-500",
    textColor: "text-green-400",
    glow: "shadow-[0_0_12px_rgba(34,197,94,0.5)]",
  },
  copied: {
    label: "Copied",
    ringColor: "ring-green-500",
    textColor: "text-green-400",
    glow: "shadow-[0_0_12px_rgba(34,197,94,0.5)]",
  },
  "note-created": {
    label: "Note Created",
    ringColor: "ring-green-500",
    textColor: "text-green-400",
    glow: "shadow-[0_0_12px_rgba(34,197,94,0.5)]",
  },
  "no-speech": {
    label: "No Speech",
    ringColor: "ring-yellow-500",
    textColor: "text-yellow-400",
    glow: "shadow-[0_0_12px_rgba(234,179,8,0.5)]",
  },
  "no-input": {
    label: "No audio detected",
    ringColor: "ring-yellow-500",
    textColor: "text-yellow-400",
    glow: "shadow-[0_0_12px_rgba(234,179,8,0.5)]",
    animate: "animate-pulse",
  },
  error: {
    label: "Failed",
    ringColor: "ring-red-500",
    textColor: "text-red-400",
    glow: "shadow-[0_0_12px_rgba(239,68,68,0.5)]",
  },
};

export function DictationBubble() {
  const [state, setState] = useState<BubbleState>("recording");
  const [slotName, setSlotName] = useState<string>("");
  useOverlayStyles();

  useEffect(() => {
    const unlisten = listenEvent(EVENTS.DICTATION_STATE, (payload) => {
      setState(payload.state);
      if (payload.slotName) {
        setSlotName(payload.slotName);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const config = stateConfig[state];
  const label = state === "recording" && slotName ? slotName : config.label;

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-transparent">
      <div className="flex items-center gap-2 rounded-full bg-background p-2 shadow-2xl border border-white/[0.08]">
        <div
          className={`flex items-center justify-center rounded-full p-1 ring-2 ${config.ringColor} ${config.glow} ${config.animate ?? ""} transition-all duration-300`}
        >
          <YapStackIcon className="h-3 w-3 text-white" />
        </div>
        <span
          className={`text-[11px] font-medium ${config.textColor} whitespace-nowrap pr-0.5 transition-colors duration-300`}
        >
          {label}
        </span>
      </div>
    </div>
  );
}
