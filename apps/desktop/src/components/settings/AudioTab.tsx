import { useState } from "react";
import { useAppStore } from "@/stores/appStore";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ChevronsUpDown } from "lucide-react";
import type { CaptureSourceDto } from "@/lib/tauri";

export function AudioTab() {
  const captureSource = useAppStore((s) => s.settings.captureSource);
  const selectedMicDeviceId = useAppStore((s) => s.settings.selectedMicDeviceId);
  const bufferMaxSeconds = useAppStore((s) => s.settings.bufferMaxSeconds);
  const mixConfig = useAppStore((s) => s.settings.mixConfig);
  const devices = useAppStore((s) => s.devices);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const inputDevices = devices.filter((d) => d.device_type === "Input");

  return (
    <>
      {/* Capture Source */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Capture Source</Label>
        <Select
          value={captureSource}
          onValueChange={(v) =>
            updateSettings({ captureSource: v as CaptureSourceDto })
          }
        >
          <SelectTrigger className="h-8 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="MicOnly" className="text-xs">
              Microphone Only
            </SelectItem>
            <SelectItem value="SystemOnly" className="text-xs">
              System Only
            </SelectItem>
            <SelectItem value="Mixed" className="text-xs">
              Mixed
            </SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Input Device */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Input Device</Label>
        <Select
          value={selectedMicDeviceId ?? ""}
          onValueChange={(v) =>
            updateSettings({ selectedMicDeviceId: v || null })
          }
        >
          <SelectTrigger className="h-8 text-xs">
            <SelectValue placeholder="Default" />
          </SelectTrigger>
          <SelectContent>
            {inputDevices.map((device) => (
              <SelectItem
                key={device.id ?? device.name}
                value={device.id ?? device.name}
                className="text-xs"
              >
                {device.name}
                {device.is_default ? " (default)" : ""}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Buffer Size */}
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Buffer Size</Label>
        <div className="flex gap-1.5">
          {[60, 120, 300, 600].map((d) => (
            <Button
              key={d}
              size="sm"
              variant={
                bufferMaxSeconds === d ? "default" : "outline"
              }
              className="flex-1 text-xs"
              onClick={() => updateSettings({ bufferMaxSeconds: d })}
            >
              {`${d / 60}m`}
            </Button>
          ))}
        </div>
        <p className="text-[10px] text-muted-foreground/60">
          Maximum audio history kept in memory for rewind
        </p>
      </div>

      <Separator />

      {/* Advanced */}
      <Collapsible open={showAdvanced} onOpenChange={setShowAdvanced}>
        <CollapsibleTrigger className="flex w-full items-center justify-between">
          <Label className="text-xs text-muted-foreground pointer-events-none">
            Advanced
          </Label>
          <ChevronsUpDown className="h-3.5 w-3.5 text-muted-foreground/60" />
        </CollapsibleTrigger>
        <CollapsibleContent className="space-y-4 pt-3">
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-[11px] text-muted-foreground">Mic Gain</span>
              <span className="font-mono text-xs text-muted-foreground">
                {mixConfig.mic_gain.toFixed(1)}
              </span>
            </div>
            <Slider
              value={[mixConfig.mic_gain]}
              min={0}
              max={2}
              step={0.1}
              onValueChange={([v]) =>
                updateSettings({
                  mixConfig: { ...mixConfig, mic_gain: v },
                })
              }
            />
          </div>
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-[11px] text-muted-foreground">System Gain</span>
              <span className="font-mono text-xs text-muted-foreground">
                {mixConfig.system_gain.toFixed(1)}
              </span>
            </div>
            <Slider
              value={[mixConfig.system_gain]}
              min={0}
              max={2}
              step={0.1}
              onValueChange={([v]) =>
                updateSettings({
                  mixConfig: { ...mixConfig, system_gain: v },
                })
              }
            />
          </div>
          <p className="text-[10px] text-muted-foreground/60">
            Gain applies to the next recording session
          </p>
        </CollapsibleContent>
      </Collapsible>
    </>
  );
}
