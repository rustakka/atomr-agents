import { useEffect, useRef, useState } from "react";
import { Check, Pencil, X } from "lucide-react";
import { cn, speakerColor } from "@/lib/utils";

interface SpeakerChipProps {
  speakerId: number;
  label: string;
  /** When false, the chip is display-only (e.g. role-based turns). */
  editable?: boolean;
  /** Commit a new label. Should trigger the optimistic store update. */
  onRename?: (label: string) => void;
  className?: string;
}

/**
 * A colored, inline-editable speaker label. Clicking the pencil swaps
 * the chip for an input; committing calls `onRename`, which the parent
 * wires to an optimistic React Query cache patch so the rename shows up
 * across every turn by that speaker at once.
 */
export function SpeakerChip({
  speakerId,
  label,
  editable = true,
  onRename,
  className,
}: SpeakerChipProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(label);
  const inputRef = useRef<HTMLInputElement>(null);
  const color = speakerColor(speakerId);

  useEffect(() => {
    setDraft(label);
  }, [label]);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    const next = draft.trim();
    if (next && next !== label) onRename?.(next);
    setEditing(false);
  };

  const cancel = () => {
    setDraft(label);
    setEditing(false);
  };

  if (editing) {
    return (
      <span className={cn("inline-flex items-center gap-1", className)}>
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") cancel();
          }}
          className="h-6 w-28 rounded-md border border-input bg-background px-1.5 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <button
          type="button"
          aria-label="save speaker name"
          onClick={commit}
          className="text-emerald-500 hover:text-emerald-400"
        >
          <Check className="size-3.5" />
        </button>
        <button
          type="button"
          aria-label="cancel"
          onClick={cancel}
          className="text-muted-foreground hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      </span>
    );
  }

  return (
    <span
      className={cn(
        "group inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium",
        className,
      )}
      style={{
        color,
        borderColor: `${color}55`,
        backgroundColor: `${color}1f`,
      }}
    >
      <span
        className="size-2 rounded-full"
        style={{ backgroundColor: color }}
        aria-hidden
      />
      {label}
      {editable && (
        <button
          type="button"
          aria-label={`rename ${label}`}
          onClick={() => setEditing(true)}
          className="opacity-0 transition-opacity group-hover:opacity-100"
        >
          <Pencil className="size-3" />
        </button>
      )}
    </span>
  );
}
