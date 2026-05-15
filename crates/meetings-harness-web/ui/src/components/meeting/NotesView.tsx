import type { Note } from "@/lib/api";
import { formatTimestamp } from "@/lib/utils";

interface NotesViewProps {
  notes: Note[];
}

/** Linear, append-only list of notes anchored to turn indices. */
export function NotesView({ notes }: NotesViewProps) {
  if (notes.length === 0) {
    return <p className="text-xs text-muted-foreground">no notes yet</p>;
  }
  return (
    <ol className="flex flex-col gap-2">
      {notes.map((note) => (
        <li
          key={note.id}
          className="rounded-md border bg-muted/30 p-2.5 text-sm leading-snug"
        >
          <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
            {note.start_ms !== null && note.start_ms !== undefined && (
              <span className="font-mono">{formatTimestamp(note.start_ms)}</span>
            )}
            {note.source_turn_indices.length > 0 && (
              <span className="font-mono">
                t#{note.source_turn_indices.join(", t#")}
              </span>
            )}
          </div>
          <p>{note.text}</p>
        </li>
      ))}
    </ol>
  );
}
