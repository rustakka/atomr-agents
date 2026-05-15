import { Radio } from "lucide-react";
import { Badge } from "@/components/ui/badge";

/** Shows whether the `/ws` live event stream is connected. */
export function LiveBadge({ connected }: { connected: boolean }) {
  return (
    <Badge variant={connected ? "success" : "outline"} className="gap-1">
      <Radio className={connected ? "size-3 animate-pulse" : "size-3"} aria-hidden />
      {connected ? "live" : "offline"}
    </Badge>
  );
}
