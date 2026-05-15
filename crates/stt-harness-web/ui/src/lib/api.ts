// Typed REST client + DTOs mirroring `atomr-agents-stt-harness`'s
// serde types. Keep in sync with `crates/stt-harness/src/conversation.rs`
// and `crates/stt-harness/src/store.rs`.

export interface Word {
  text: string;
  start_ms: number;
  end_ms: number;
  confidence: number | null;
}

export type SpeakerRef =
  | { kind: "diarized"; tag: { id: number; label: string | null } }
  | { kind: "role"; role: "system" | "user" | "assistant" | "tool" }
  | { kind: "unknown" };

export interface SttTurn {
  index: number;
  speaker: SpeakerRef;
  text: string;
  start_ms: number;
  end_ms: number;
  words: Word[];
  confidence: number | null;
  state: "partial" | "final";
}

export interface SttConversation {
  id: string;
  language: string | null;
  turns: SttTurn[];
  backend: string | null;
  model_id: string | null;
  total_audio_secs: number;
  speaker_labels: Record<string, string>;
}

export interface ConversationSummary {
  id: string;
  language: string | null;
  turn_count: number;
  speaker_count: number;
  total_audio_secs: number;
  backend: string | null;
}

/** The numeric diarized speaker id for a turn, or null. */
export function turnSpeakerId(turn: SttTurn): number | null {
  return turn.speaker.kind === "diarized" ? turn.speaker.tag.id : null;
}

/** Resolve a speaker's display label: per-conversation override first,
 *  then the `speaker_{id}` fallback. Mirrors
 *  `SttConversation::effective_label`. */
export function effectiveLabel(
  conv: Pick<SttConversation, "speaker_labels">,
  speakerId: number,
): string {
  return conv.speaker_labels[String(speakerId)] ?? `speaker_${speakerId}`;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, { credentials: "same-origin", ...init });
  if (!resp.ok) {
    let detail = `${resp.status} ${resp.statusText}`;
    try {
      const body = await resp.json();
      if (body?.error) detail = body.error;
    } catch {
      // non-JSON error body — keep the status line
    }
    throw new Error(detail);
  }
  if (resp.status === 204) return undefined as T;
  return resp.json() as Promise<T>;
}

export const api = {
  listConversations: () =>
    request<ConversationSummary[]>("/api/conversations"),

  getConversation: (id: string) =>
    request<SttConversation>(`/api/conversations/${encodeURIComponent(id)}`),

  deleteConversation: (id: string) =>
    request<void>(`/api/conversations/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),

  renameSpeaker: (id: string, speakerId: number, label: string) =>
    request<SttConversation>(
      `/api/conversations/${encodeURIComponent(id)}/speakers/${speakerId}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ label }),
      },
    ),
};
