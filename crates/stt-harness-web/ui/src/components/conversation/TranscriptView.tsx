import {
  effectiveLabel,
  turnSpeakerId,
  type SttConversation,
  type SttTurn,
} from "@/lib/api";
import { cn, formatTimestamp, speakerColor } from "@/lib/utils";
import { SpeakerChip } from "./SpeakerChip";

interface TranscriptViewProps {
  conversation: SttConversation;
  onRename: (speakerId: number, label: string) => void;
}

function turnLabel(conversation: SttConversation, turn: SttTurn): string {
  const id = turnSpeakerId(turn);
  if (id !== null) return effectiveLabel(conversation, id);
  if (turn.speaker.kind === "role") return turn.speaker.role;
  return "unknown";
}

/** The ordered transcript: one row per turn, speaker chip + text +
 *  timing. Editing a speaker's chip renames every turn by that speaker. */
export function TranscriptView({ conversation, onRename }: TranscriptViewProps) {
  if (conversation.turns.length === 0) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        transcript is empty
      </p>
    );
  }

  return (
    <ol className="flex flex-col gap-3">
      {conversation.turns.map((turn) => {
        const speakerId = turnSpeakerId(turn);
        const color = speakerId !== null ? speakerColor(speakerId) : undefined;
        return (
          <li
            key={turn.index}
            className="flex gap-3 rounded-md border-l-2 bg-card/40 p-3"
            style={color ? { borderLeftColor: color } : undefined}
          >
            <div className="flex w-32 shrink-0 flex-col gap-1">
              {speakerId !== null ? (
                <SpeakerChip
                  speakerId={speakerId}
                  label={turnLabel(conversation, turn)}
                  onRename={(label) => onRename(speakerId, label)}
                />
              ) : (
                <span className="text-xs text-muted-foreground">
                  {turnLabel(conversation, turn)}
                </span>
              )}
              <span className="text-[10px] tabular-nums text-muted-foreground">
                {formatTimestamp(turn.start_ms)}
                {turn.end_ms > turn.start_ms && ` – ${formatTimestamp(turn.end_ms)}`}
              </span>
            </div>
            <p
              className={cn(
                "min-w-0 flex-1 whitespace-pre-wrap text-sm leading-relaxed",
                turn.state === "partial" && "italic text-muted-foreground",
              )}
            >
              {turn.text}
            </p>
          </li>
        );
      })}
    </ol>
  );
}
