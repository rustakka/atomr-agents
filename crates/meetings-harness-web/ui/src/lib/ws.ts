// Reconnecting WebSocket hook for the `/ws` meetings-harness event stream.

import { useEffect, useRef } from "react";
import type { Action, Attendee, Note, SegmentSummary } from "./api";

/** A live event from the meetings harness, as serialized by
 *  `MeetingsHarnessEvent` (`#[serde(tag = "kind")]`). */
export type MeetingsHarnessEvent =
  | { kind: "started"; mode: string; source_transcript_id: string }
  | { kind: "attendee_upserted"; attendee: Attendee }
  | { kind: "note_appended"; note: Note }
  | { kind: "action_appended"; action: Action }
  | {
      kind: "action_updated";
      action_id: string;
      status?: Action["status"] | null;
      owner_attendee_id?: string | null;
      due_iso?: string | null;
    }
  | { kind: "segment_revised"; segment: SegmentSummary }
  | { kind: "segment_finalized"; segment: SegmentSummary }
  | { kind: "running_summary_updated"; text: string }
  | { kind: "title_set"; title: string }
  | { kind: "watermark_advanced"; turn_index: number }
  | { kind: "progress"; processed: number; total: number }
  | { kind: "finalized"; reason: string; note_count: number; action_count: number }
  | { kind: "stopped"; reason: string }
  | { kind: "error"; detail: string };

export interface WsOptions {
  onEvent: (event: MeetingsHarnessEvent) => void;
  onStatusChange?: (connected: boolean) => void;
  enabled?: boolean;
}

/** Subscribe to the `/ws` stream for the lifetime of the component. */
export function useMeetingsStream({
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
          const msg = JSON.parse(ev.data) as MeetingsHarnessEvent;
          onEventRef.current(msg);
        } catch {
          // ignore non-event frames (e.g. `{"kind":"lagged"}`)
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
