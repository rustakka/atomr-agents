import { prettyJson } from "@/lib/utils";
import { cn } from "@/lib/utils";

/** Read-only pretty-printed JSON block. */
export function JsonView({ value, className }: { value: unknown; className?: string }) {
  return (
    <pre
      className={cn(
        "max-h-[60vh] overflow-auto rounded-md border bg-muted/40 p-3 text-xs leading-relaxed",
        className,
      )}
    >
      <code>{prettyJson(value)}</code>
    </pre>
  );
}
