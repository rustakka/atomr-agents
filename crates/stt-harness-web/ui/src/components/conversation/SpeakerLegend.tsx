import { effectiveLabel, type SttConversation } from "@/lib/api";
import { SpeakerChip } from "./SpeakerChip";

interface SpeakerLegendProps {
  conversation: SttConversation;
  onRename: (speakerId: number, label: string) => void;
}

/** The roster of diarized speakers in a conversation, each editable. */
export function SpeakerLegend({ conversation, onRename }: SpeakerLegendProps) {
  const ids = Array.from(
    new Set(
      conversation.turns
        .map((t) => (t.speaker.kind === "diarized" ? t.speaker.tag.id : null))
        .filter((id): id is number => id !== null),
    ),
  ).sort((a, b) => a - b);

  if (ids.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        No diarized speakers in this conversation.
      </p>
    );
  }

  return (
    <div className="flex flex-wrap gap-2">
      {ids.map((id) => (
        <SpeakerChip
          key={id}
          speakerId={id}
          label={effectiveLabel(conversation, id)}
          onRename={(label) => onRename(id, label)}
        />
      ))}
    </div>
  );
}
