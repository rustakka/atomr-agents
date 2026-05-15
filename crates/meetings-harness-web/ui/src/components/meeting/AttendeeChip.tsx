import { useEffect, useRef, useState } from "react";
import { Check, Pencil, X } from "lucide-react";
import { cn, attendeeColor } from "@/lib/utils";
import type { Attendee } from "@/lib/api";

interface AttendeeChipProps {
  attendee: Attendee;
  editable?: boolean;
  onRename?: (displayName: string) => void;
  className?: string;
}

/**
 * Colored, inline-editable attendee chip. Mirrors the SpeakerChip
 * pattern from the STT review UI.
 */
export function AttendeeChip({
  attendee,
  editable = true,
  onRename,
  className,
}: AttendeeChipProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(attendee.display_name);
  const inputRef = useRef<HTMLInputElement>(null);
  const primaryTag = attendee.speaker_tags[0] ?? 0;
  const color = attendeeColor(primaryTag);

  useEffect(() => {
    setDraft(attendee.display_name);
  }, [attendee.display_name]);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    const next = draft.trim();
    if (next && next !== attendee.display_name) onRename?.(next);
    setEditing(false);
  };

  const cancel = () => {
    setDraft(attendee.display_name);
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
          className="h-6 w-32 rounded-md border border-input bg-background px-1.5 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <button
          type="button"
          aria-label="save attendee name"
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
      style={{ color, borderColor: `${color}55`, backgroundColor: `${color}1f` }}
    >
      <span
        className="size-2 rounded-full"
        style={{ backgroundColor: color }}
        aria-hidden
      />
      {attendee.display_name}
      {attendee.role && (
        <span className="ml-1 text-muted-foreground/80">· {attendee.role}</span>
      )}
      {editable && (
        <button
          type="button"
          aria-label={`rename ${attendee.display_name}`}
          onClick={() => setEditing(true)}
          className="opacity-0 transition-opacity group-hover:opacity-100"
        >
          <Pencil className="size-3" />
        </button>
      )}
    </span>
  );
}
