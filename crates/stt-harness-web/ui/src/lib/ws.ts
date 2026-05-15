// Reconnecting WebSocket hook for the `/ws` STT-harness event stream.
// Adapted from atomr-dashboard's `ws.ts`.

import { useEffect, useRef } from "react";
import type { SttTurn } from "./api";

/** A live event from the STT harness, as serialized by
 *  `SttHarnessEvent` (`#[serde(tag = "kind")]`). */
export type SttHarnessEvent =
  | { kind: "started"; backend: string; diarization: string }
  | { kind: "partial"; text: string; start_ms: number; end_ms: number }
  | { kind: "utterance_committed"; turn: SttTurn }
  | { kind: "speaker_changed"; speaker: { id: number; label: string | null }; at_ms: number }
  | { kind: "utterance_end"; at_ms: number }
  | { kind: "metadata"; data: unknown }
  | { kind: "diarization_warning"; detail: string }
  | { kind: "finished"; reason: string; turn_count: number; total_audio_secs: number }
  | { kind: "error"; detail: string };

export interface WsOptions {
  onEvent: (event: SttHarnessEvent) => void;
  onStatusChange?: (connected: boolean) => void;
  enabled?: boolean;
}

/** Subscribe to the `/ws` stream for the lifetime of the component. */
export function useHarnessStream({
  onEvent,
  onStatusChange,
  enabled = true,
}: WsOptions): void {
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  const onStatusRef = useRef(onStatusChange);
  onStatusRef.current = onStatusChange;

  useEffect(() => {
    if (!enabled) return;
    let closed = false;
    let socket: WebSocket | null = null;
    let attempt = 0;

    const connect = () => {
      const proto = window.location.protocol === "https:" ? "wss" : "ws";
      const url = `${proto}://${window.location.host}/ws`;
      socket = new WebSocket(url);
      socket.onopen = () => {
        attempt = 0;
        onStatusRef.current?.(true);
      };
      socket.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data) as SttHarnessEvent;
          onEventRef.current(msg);
        } catch {
          // ignore non-JSON frames (e.g. `{"kind":"lagged"}` notices)
        }
      };
      socket.onclose = () => {
        onStatusRef.current?.(false);
        if (closed) return;
        attempt += 1;
        const delay = Math.min(30_000, 500 * 2 ** Math.min(attempt, 6));
        setTimeout(connect, delay);
      };
      socket.onerror = () => socket?.close();
    };

    connect();
    return () => {
      closed = true;
      socket?.close();
    };
  }, [enabled]);
}
