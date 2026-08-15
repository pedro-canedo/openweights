import { useEffect, useState } from "react";
import { listen } from "../lib/tauri";
import type { Telemetry } from "../lib/types";

export function useTelemetry(): Telemetry | null {
  const [telemetry, setTelemetry] = useState<Telemetry | null>(null);

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    listen<Telemetry>("telemetry", setTelemetry).then((f) => {
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  return telemetry;
}
