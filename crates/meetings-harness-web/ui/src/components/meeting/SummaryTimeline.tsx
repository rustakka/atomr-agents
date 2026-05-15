import type { SegmentSummary } from "@/lib/api";
import { Badge } from "@/components/ui/badge";

interface SummaryTimelineProps {
  segments: SegmentSummary[];
}

/** Collapsible list of per-segment summaries — the unfinalized tail is
 *  marked "in-flight" and visibly updates as new turns commit. */
export function SummaryTimeline({ segments }: SummaryTimelineProps) {
  if (segments.length === 0) {
    return <p className="text-xs text-muted-foreground">no segments yet</p>;
  }
  return (
    <ul className="flex flex-col gap-2">
      {segments.map((s) => (
        <li
          key={s.id}
          className="rounded-md border bg-muted/30 p-2.5 text-sm leading-snug"
        >
          <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
            <span className="font-mono">
              t#{s.start_turn_index}–t#{s.end_turn_index}
            </span>
            {s.finalized ? (
              <Badge variant="outline">finalized</Badge>
            ) : (
              <Badge variant="warning">in-flight</Badge>
            )}
          </div>
          <p>{s.text}</p>
        </li>
      ))}
    </ul>
  );
}
