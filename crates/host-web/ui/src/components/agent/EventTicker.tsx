import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { cn, formatRelativeMs } from "@/lib/utils";
import { api } from "@/lib/api";
import { useHostEvents } from "@/lib/sse";
import { Badge } from "@/components/ui/badge";
import type { EventRecord } from "@/lib/apiTypes";

/** Live ticker of the most recent host events (seeded from history + SSE). */
export function EventTicker({ limit = 10 }: { limit?: number }) {
  const [events, setEvents] = useState<EventRecord[]>([]);
  const [connected, setConnected] = useState(false);

  const history = useQuery({
    queryKey: ["events", "ticker", limit],
    queryFn: () => api.listEvents(limit),
  });

  useEffect(() => {
    if (history.data) setEvents(history.data.events.slice(0, limit));
  }, [history.data, limit]);

  useHostEvents({
    onEvent: (ev) => setEvents((prev) => [ev, ...prev].slice(0, limit)),
    onStatusChange: setConnected,
  });

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium text-muted-foreground">Events</span>
        <span
          className={cn(
            "size-2 rounded-full",
            connected ? "bg-emerald-500" : "bg-muted-foreground/40",
          )}
          title={connected ? "live" : "disconnected"}
        />
      </div>
      {events.length === 0 ? (
        <p className="text-xs text-muted-foreground">No events yet.</p>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {events.map((ev, i) => (
            <li key={`${ev.ts_ms}-${i}`} className="flex items-center gap-2 text-xs">
              <Badge variant="outline" className="shrink-0">
                {ev.kind}
              </Badge>
              {ev.agent_id && (
                <span className="shrink-0 text-muted-foreground">{ev.agent_id}</span>
              )}
              <span className="ml-auto shrink-0 text-muted-foreground/70">
                {formatRelativeMs(ev.ts_ms)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
