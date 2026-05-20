// Reconnecting EventSource hook for the host `/api/events/stream` SSE feed.
// Each message arrives as event name `host_event` with a JSON `EventRecord`
// payload in `data`.

import { useEffect, useRef } from "react";
import type { EventRecord } from "./apiTypes";

export interface SseOptions {
  onEvent: (event: EventRecord) => void;
  onStatusChange?: (connected: boolean) => void;
  enabled?: boolean;
}

/** Subscribe to the host event stream for the lifetime of the component. */
export function useHostEvents({
  onEvent,
  onStatusChange,
  enabled = true,
}: SseOptions): void {
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  const onStatusRef = useRef(onStatusChange);
  onStatusRef.current = onStatusChange;

  useEffect(() => {
    if (!enabled) return;
    let closed = false;
    let source: EventSource | null = null;
    let retry: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;

    const handle = (ev: MessageEvent) => {
      try {
        const record = JSON.parse(ev.data) as EventRecord;
        onEventRef.current(record);
      } catch {
        // ignore non-JSON frames (e.g. keep-alive comments)
      }
    };

    const connect = () => {
      source = new EventSource("/api/events/stream", {
        withCredentials: true,
      });
      source.addEventListener("open", () => {
        attempt = 0;
        onStatusRef.current?.(true);
      });
      // Named SSE event used by the backend.
      source.addEventListener("host_event", handle as EventListener);
      // Fallback: unnamed `message` events.
      source.addEventListener("message", handle as EventListener);
      source.addEventListener("error", () => {
        onStatusRef.current?.(false);
        source?.close();
        if (closed) return;
        attempt += 1;
        const delay = Math.min(30_000, 500 * 2 ** Math.min(attempt, 6));
        retry = setTimeout(connect, delay);
      });
    };

    connect();
    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      source?.close();
    };
  }, [enabled]);
}
