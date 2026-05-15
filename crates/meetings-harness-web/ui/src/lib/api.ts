// Typed REST client + DTOs mirroring `atomr-agents-meetings-harness`'s
// serde types. Keep in sync with `crates/meetings-harness/src/analysis.rs`
// and `crates/meetings-harness/src/store.rs`.

export type ActionStatus = "open" | "done" | "cancelled";
export type AnalysisState = "pending" | "streaming" | "final";

export interface Attendee {
  id: string;
  display_name: string;
  role?: string | null;
  speaker_tags: number[];
  email?: string | null;
}

export interface Note {
  id: string;
  text: string;
  source_turn_indices: number[];
  start_ms?: number | null;
  end_ms?: number | null;
}

export interface Action {
  id: string;
  description: string;
  owner_attendee_id?: string | null;
  due_iso?: string | null;
  supporting_quote?: string | null;
  source_turn_index?: number | null;
  status: ActionStatus;
}

export interface SegmentSummary {
  id: string;
  start_turn_index: number;
  end_turn_index: number;
  text: string;
  finalized: boolean;
}

export interface SummaryLevels {
  segments: SegmentSummary[];
  running?: string | null;
  tldr?: string | null;
}

export interface MeetingAnalysis {
  id: string;
  title?: string | null;
  summary_levels: SummaryLevels;
  attendees: Attendee[];
  notes: Note[];
  actions: Action[];
  source_transcript_id: string;
  last_processed_turn_index?: number | null;
  generated_at_ms: number;
  updated_at_ms: number;
  model_id?: string | null;
  state: AnalysisState;
}

export interface MeetingsSummary {
  id: string;
  title?: string | null;
  attendee_count: number;
  note_count: number;
  action_count: number;
  open_action_count: number;
  state: AnalysisState;
  generated_at_ms: number;
  updated_at_ms: number;
}

export interface ConversationSummary {
  id: string;
  language: string | null;
  turn_count: number;
  speaker_count: number;
  total_audio_secs: number;
  backend: string | null;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, { credentials: "same-origin", ...init });
  if (!resp.ok) {
    let detail = `${resp.status} ${resp.statusText}`;
    try {
      const body = await resp.json();
      if (body?.error) detail = body.error;
    } catch {
      // non-JSON error body
    }
    throw new Error(detail);
  }
  if (resp.status === 204) return undefined as T;
  return resp.json() as Promise<T>;
}

export const api = {
  listMeetings: () => request<MeetingsSummary[]>("/api/meetings"),

  getMeeting: (id: string) =>
    request<MeetingAnalysis>(`/api/meetings/${encodeURIComponent(id)}`),

  deleteMeeting: (id: string) =>
    request<void>(`/api/meetings/${encodeURIComponent(id)}`, { method: "DELETE" }),

  renameAttendee: (
    id: string,
    attendeeId: string,
    body: { display_name?: string; role?: string | null; email?: string | null },
  ) =>
    request<MeetingAnalysis>(
      `/api/meetings/${encodeURIComponent(id)}/attendees/${encodeURIComponent(attendeeId)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      },
    ),

  updateAction: (
    id: string,
    actionId: string,
    body: {
      status?: ActionStatus;
      owner_attendee_id?: string | null;
      due_iso?: string | null;
    },
  ) =>
    request<MeetingAnalysis>(
      `/api/meetings/${encodeURIComponent(id)}/actions/${encodeURIComponent(actionId)}`,
      {
        method: "PATCH",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      },
    ),

  triggerRun: (
    id: string,
    body: {
      mode: "batch" | "live";
      model_id: string;
      max_iterations?: number;
      segment_turn_count?: number;
    },
  ) =>
    request<MeetingAnalysis>(`/api/meetings/${encodeURIComponent(id)}/run`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),

  stopRun: (id: string) =>
    request<void>(`/api/meetings/${encodeURIComponent(id)}/stop`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    }),

  listTranscripts: () => request<ConversationSummary[]>("/api/transcripts"),
};
