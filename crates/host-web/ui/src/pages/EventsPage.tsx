import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { History } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/states";
import { JsonView } from "@/components/ui/json-view";
import { useHostEvents } from "@/lib/sse";
import { cn, formatRelativeMs } from "@/lib/utils";
import type { EventRecord } from "@/lib/apiTypes";

export default function EventsPage() {
  const [events, setEvents] = useState<EventRecord[]>([]);
  const [connected, setConnected] = useState(false);

  // Lazy-load history on demand rather than on mount.
  const history = useQuery({
    queryKey: ["events", "history"],
    queryFn: () => api.listEvents(200),
    enabled: false,
  });

  useEffect(() => {
    if (history.data) {
      setEvents((prev) => {
        // Merge history under any live events already received, dedup-free
        // since records are append-only and ordered newest-first.
        const seen = new Set(prev.map((e) => `${e.ts_ms}:${e.kind}`));
        const merged = [...prev];
        for (const e of history.data.events) {
          if (!seen.has(`${e.ts_ms}:${e.kind}`)) merged.push(e);
        }
        return merged.sort((a, b) => b.ts_ms - a.ts_ms);
      });
    }
  }, [history.data]);

  useHostEvents({
    onEvent: (ev) => setEvents((prev) => [ev, ...prev].slice(0, 500)),
    onStatusChange: setConnected,
  });

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4">
      <div className="flex items-center gap-2">
        <h1 className="text-lg font-semibold">Events</h1>
        <span
          className={cn(
            "size-2 rounded-full",
            connected ? "bg-emerald-500" : "bg-muted-foreground/40",
          )}
          title={connected ? "live" : "disconnected"}
        />
        <Button
          size="sm"
          variant="outline"
          className="ml-auto"
          disabled={history.isFetching}
          onClick={() => history.refetch()}
        >
          <History className="size-3.5" /> Load history
        </Button>
      </div>

      {events.length === 0 ? (
        <EmptyState title="No events yet" hint="Streaming live from the host." />
      ) : (
        <div className="flex flex-col gap-2">
          {events.map((ev, i) => (
            <Card key={`${ev.ts_ms}-${i}`}>
              <CardContent className="flex flex-col gap-2 pt-4">
                <div className="flex flex-wrap items-center gap-2 text-xs">
                  <Badge variant="outline">{ev.kind}</Badge>
                  {ev.agent_id && (
                    <Badge variant="default">{ev.agent_id}</Badge>
                  )}
                  <span className="ml-auto text-muted-foreground">
                    {formatRelativeMs(ev.ts_ms)}
                  </span>
                </div>
                <JsonView value={ev.payload} className="max-h-40" />
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
