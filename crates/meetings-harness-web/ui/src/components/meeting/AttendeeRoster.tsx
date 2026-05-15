import type { Attendee } from "@/lib/api";
import { AttendeeChip } from "./AttendeeChip";

interface AttendeeRosterProps {
  attendees: Attendee[];
  onRename: (attendeeId: string, displayName: string) => void;
}

export function AttendeeRoster({ attendees, onRename }: AttendeeRosterProps) {
  if (attendees.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">no attendees identified yet</p>
    );
  }
  return (
    <div className="flex flex-wrap gap-1.5">
      {attendees.map((a) => (
        <AttendeeChip
          key={a.id}
          attendee={a}
          onRename={(name) => onRename(a.id, name)}
        />
      ))}
    </div>
  );
}
