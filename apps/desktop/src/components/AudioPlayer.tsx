import { useState, useRef, useEffect, useCallback } from "react";
import { Slider } from "@/components/ui/slider";
import { Button } from "@/components/ui/button";
import { Play, Pause } from "lucide-react";
import { formatTime } from "@/lib/utils";
import { trackAudioPlaybackStarted } from "@/lib/analytics";

const PLAYBACK_SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2] as const;

export function AudioPlayer({
  src,
  duration,
  onTimeUpdate,
  onPlayStateChange,
  onDurationResolved,
}: {
  src: string;
  duration: number;
  onTimeUpdate?: (time: number) => void;
  onPlayStateChange?: (playing: boolean) => void;
  onDurationResolved?: (d: number) => void;
}) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [speed, setSpeed] = useState(1);
  const [resolvedDuration, setResolvedDuration] = useState(duration);
  const rafRef = useRef<number>(0);
  const lastEmitRef = useRef(0);

  // Sync resolvedDuration when prop changes (if prop > 0)
  useEffect(() => {
    if (duration > 0) {
      setResolvedDuration(duration);
    }
  }, [duration]);

  const updateTime = useCallback(() => {
    if (audioRef.current) {
      const t = audioRef.current.currentTime;
      setCurrentTime(t); // 60fps for smooth slider
      // Throttle store updates to ~4Hz
      const now = performance.now();
      if (now - lastEmitRef.current >= 250) {
        onTimeUpdate?.(t);
        lastEmitRef.current = now;
      }
    }
    if (isPlaying) {
      rafRef.current = requestAnimationFrame(updateTime);
    }
  }, [isPlaying, onTimeUpdate]);

  useEffect(() => {
    if (isPlaying) {
      rafRef.current = requestAnimationFrame(updateTime);
    }
    return () => cancelAnimationFrame(rafRef.current);
  }, [isPlaying, updateTime]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    const handleEnded = () => {
      setIsPlaying(false);
      onTimeUpdate?.(audio.currentTime);
      onPlayStateChange?.(false);
    };

    // Sync local state when external code seeks (e.g. timestamp click)
    const handleSeeked = () => {
      if (audio) {
        setCurrentTime(audio.currentTime);
        onTimeUpdate?.(audio.currentTime);
      }
    };

    // Sync play state when external code starts playback
    const handlePlay = () => {
      setIsPlaying(true);
      onPlayStateChange?.(true);
    };

    // Use audio element's duration as fallback when prop is 0
    const handleLoadedMetadata = () => {
      if (audio.duration && isFinite(audio.duration) && audio.duration > 0 && resolvedDuration === 0) {
        setResolvedDuration(audio.duration);
        onDurationResolved?.(audio.duration);
      }
    };

    audio.addEventListener("ended", handleEnded);
    audio.addEventListener("seeked", handleSeeked);
    audio.addEventListener("play", handlePlay);
    audio.addEventListener("loadedmetadata", handleLoadedMetadata);
    return () => {
      audio.removeEventListener("ended", handleEnded);
      audio.removeEventListener("seeked", handleSeeked);
      audio.removeEventListener("play", handlePlay);
      audio.removeEventListener("loadedmetadata", handleLoadedMetadata);
    };
  }, [onPlayStateChange, onTimeUpdate, onDurationResolved, resolvedDuration]);

  const togglePlay = () => {
    const audio = audioRef.current;
    if (!audio) return;

    if (isPlaying) {
      audio.pause();
      setIsPlaying(false);
      onTimeUpdate?.(audio.currentTime);
      onPlayStateChange?.(false);
    } else {
      audio.play().catch((e) => {
        console.error("Audio playback failed:", e);
        setIsPlaying(false);
        onPlayStateChange?.(false);
      });
      setIsPlaying(true);
      onPlayStateChange?.(true);
      trackAudioPlaybackStarted({ duration_seconds: Math.round(resolvedDuration) });
    }
  };

  const handleSeek = useCallback(
    (value: number[]) => {
      const audio = audioRef.current;
      if (!audio) return;
      audio.currentTime = value[0];
      setCurrentTime(value[0]);
      onTimeUpdate?.(value[0]);
    },
    [onTimeUpdate],
  );

  const cycleSpeed = () => {
    const idx = PLAYBACK_SPEEDS.indexOf(speed as (typeof PLAYBACK_SPEEDS)[number]);
    const nextIdx = (idx + 1) % PLAYBACK_SPEEDS.length;
    const next = PLAYBACK_SPEEDS[nextIdx];
    setSpeed(next);
    if (audioRef.current) {
      audioRef.current.playbackRate = next;
    }
  };

  // Seek to a specific time (called externally via ref or parent)
  const seekTo = useCallback(
    (time: number) => {
      const audio = audioRef.current;
      if (!audio) return;
      audio.currentTime = time;
      setCurrentTime(time);
      onTimeUpdate?.(time);
    },
    [onTimeUpdate],
  );

  // Expose seekTo via a stable callback the parent can use
  useEffect(() => {
    // Store on the audio element for parent access
    if (audioRef.current) {
      (audioRef.current as HTMLAudioElement & { seekTo?: (t: number) => void }).seekTo = seekTo;
    }
  }, [seekTo]);

  return (
    <div className="flex items-center gap-3 border-b px-4 py-2">
      <audio ref={audioRef} src={src} preload="metadata" data-session-audio />

      <Button variant="ghost" size="icon-xs" onClick={togglePlay}>
        {isPlaying ? (
          <Pause className="h-4 w-4" />
        ) : (
          <Play className="h-4 w-4" />
        )}
      </Button>

      <span className="text-xs tabular-nums text-muted-foreground w-10 text-right">
        {formatTime(currentTime)}
      </span>

      <Slider
        className="flex-1"
        min={0}
        max={resolvedDuration}
        step={0.1}
        value={[currentTime]}
        onValueChange={handleSeek}
      />

      <span className="text-xs tabular-nums text-muted-foreground w-10">
        {formatTime(resolvedDuration)}
      </span>

      <button
        onClick={cycleSpeed}
        className="text-xs tabular-nums text-muted-foreground hover:text-foreground transition-colors min-w-[2.5rem] text-center"
      >
        {speed}x
      </button>
    </div>
  );
}
