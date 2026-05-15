import { useCallback, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ArrowLeft, Square } from "lucide-react";
import {
  api,
  type ActionStatus,
  type MeetingAnalysis,
} from "@/lib/api";
import { useMeetingsStream } from "@/lib/ws";
import { formatRelativeMs } from "@/lib/utils";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { LiveBadge } from "@/components/meeting/LiveBadge";
import { AttendeeRoster } from "@/components/meeting/AttendeeRoster";
import { NotesView } from "@/components/meeting/NotesView";
import { ActionsTable } from "@/components/meeting/ActionsTable";
import { SummaryTimeline } from "@/components/meeting/SummaryTimeline";

/** One meeting analysis: tiered summaries + attendee/notes/actions
 *  ledgers + live updates. */
export default function MeetingDetailPage() {
  const { id = "" } = useParams<{ id: string }>();
  const queryClient = useQueryClient();
  const queryKey = ["meeting", id] as const;

  const { data: analysis, isLoading, error } = useQuery({
    queryKey,
    queryFn: () => api.getMeeting(id),
  });

  const [connected, setConnected] = useState(false);
  useMeetingsStream({
    onStatusChange: setConnected,
    onEvent: (ev) => {
      // Invalidate on any structural change.
      switch (ev.kind) {
        case "attendee_upserted":
        case "note_appended":
        case "action_appended":
        case "action_updated":
        case "segment_revised":
        case "segment_finalized":
        case "running_summary_updated":
        case "title_set":
        case "finalized":
        case "stopped":
          queryClient.invalidateQueries({ queryKey });
          queryClient.invalidateQueries({ queryKey: ["meetings"] });
          break;
        default:
          break;
      }
    },
  });

  const renameAttendee = useMutation({
    mutationFn: ({
      attendeeId,
      displayName,
    }: {
      attendeeId: string;
      displayName: string;
    }) => api.renameAttendee(id, attendeeId, { display_name: displayName }),
    onSuccess: (updated) => queryClient.setQueryData(queryKey, updated),
  });

  const patchAction = useMutation({
    mutationFn: ({
      actionId,
      body,
    }: {
      actionId: string;
      body: { status?: ActionStatus; owner_attendee_id?: string | null };
    }) => api.updateAction(id, actionId, body),
    onMutate: async ({ actionId, body }) => {
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData<MeetingAnalysis>(queryKey);
      if (previous) {
        const next: MeetingAnalysis = {
          ...previous,
          actions: previous.actions.map((a) =>
            a.id === actionId ? { ...a, ...body } : a,
          ),
        };
        queryClient.setQueryData<MeetingAnalysis>(queryKey, next);
      }
      return { previous };
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.previous) queryClient.setQueryData(queryKey, ctx.previous);
    },
    onSuccess: (updated) => queryClient.setQueryData(queryKey, updated),
  });

  const onRename = useCallback(
    (attendeeId: string, displayName: string) =>
      renameAttendee.mutate({ attendeeId, displayName }),
    [renameAttendee],
  );

  const onUpdateAction = useCallback(
    (
      actionId: string,
      body: { status?: ActionStatus; owner_attendee_id?: string | null },
    ) => patchAction.mutate({ actionId, body }),
    [patchAction],
  );

  const onStop = useCallback(async () => {
    await api.stopRun(id);
  }, [id]);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <Link
          to="/"
          className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-4" />
          Meetings
        </Link>
        <LiveBadge connected={connected} />
        {analysis?.state === "streaming" && (
          <Button size="sm" variant="outline" onClick={onStop}>
            <Square className="size-3.5" />
            Stop
          </Button>
        )}
      </div>

      {isLoading && <Skeleton className="h-64 w-full" />}
      {error && (
        <Card>
          <CardContent className="py-6 text-center text-destructive">
            {(error as Error).message}
          </CardContent>
        </Card>
      )}

      {analysis && (
        <>
          <Card>
            <CardHeader className="gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <CardTitle className="font-mono">{analysis.id}</CardTitle>
                <Badge
                  variant={
                    analysis.state === "final"
                      ? "success"
                      : analysis.state === "streaming"
                        ? "warning"
                        : "outline"
                  }
                >
                  {analysis.state}
                </Badge>
                {analysis.title && <Badge variant="outline">{analysis.title}</Badge>}
                {analysis.model_id && (
                  <Badge variant="outline">model: {analysis.model_id}</Badge>
                )}
                <span className="text-xs text-muted-foreground">
                  updated {formatRelativeMs(analysis.updated_at_ms)}
                </span>
              </div>
              <AttendeeRoster
                attendees={analysis.attendees}
                onRename={onRename}
              />
            </CardHeader>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Summary</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              {analysis.summary_levels.tldr && (
                <div>
                  <p className="text-xs uppercase text-muted-foreground">TL;DR</p>
                  <p className="text-sm">{analysis.summary_levels.tldr}</p>
                </div>
              )}
              {analysis.summary_levels.running && (
                <div>
                  <p className="text-xs uppercase text-muted-foreground">
                    Running rollup
                  </p>
                  <p className="whitespace-pre-line text-sm">
                    {analysis.summary_levels.running}
                  </p>
                </div>
              )}
              <div>
                <p className="text-xs uppercase text-muted-foreground">Segments</p>
                <SummaryTimeline segments={analysis.summary_levels.segments} />
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>
                Actions <Badge variant="outline">{analysis.actions.length}</Badge>
              </CardTitle>
            </CardHeader>
            <CardContent>
              <ActionsTable
                actions={analysis.actions}
                attendees={analysis.attendees}
                onUpdate={onUpdateAction}
              />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>
                Notes <Badge variant="outline">{analysis.notes.length}</Badge>
              </CardTitle>
            </CardHeader>
            <CardContent>
              <NotesView notes={analysis.notes} />
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
