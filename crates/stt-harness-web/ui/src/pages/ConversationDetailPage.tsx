import { useCallback, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { api, type SttConversation } from "@/lib/api";
import { useHarnessStream } from "@/lib/ws";
import { formatDuration } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { TranscriptView } from "@/components/conversation/TranscriptView";
import { SpeakerLegend } from "@/components/conversation/SpeakerLegend";
import { LiveBadge } from "@/components/conversation/LiveBadge";

/** One conversation: editable diarized transcript + live updates. */
export default function ConversationDetailPage() {
  const { id = "" } = useParams<{ id: string }>();
  const queryClient = useQueryClient();
  const queryKey = ["conversation", id] as const;

  const { data: conversation, isLoading, error } = useQuery({
    queryKey,
    queryFn: () => api.getConversation(id),
  });

  // Live event stream — drives the connection badge and refreshes the
  // transcript as utterances commit. `partial` events feed an interim
  // preview line shown beneath the committed turns.
  const [connected, setConnected] = useState(false);
  const [partial, setPartial] = useState<string | null>(null);
  useHarnessStream({
    onStatusChange: setConnected,
    onEvent: (ev) => {
      switch (ev.kind) {
        case "partial":
          setPartial(ev.text);
          break;
        case "utterance_committed":
          setPartial(null);
          queryClient.invalidateQueries({ queryKey });
          queryClient.invalidateQueries({ queryKey: ["conversations"] });
          break;
        case "finished":
          setPartial(null);
          queryClient.invalidateQueries({ queryKey });
          break;
        default:
          break;
      }
    },
  });

  // Optimistic speaker rename: patch `speaker_labels` in the cache
  // immediately so every turn by that speaker re-labels at once, then
  // reconcile with the server response (or roll back on error).
  const rename = useMutation({
    mutationFn: ({ speakerId, label }: { speakerId: number; label: string }) =>
      api.renameSpeaker(id, speakerId, label),
    onMutate: async ({ speakerId, label }) => {
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData<SttConversation>(queryKey);
      if (previous) {
        queryClient.setQueryData<SttConversation>(queryKey, {
          ...previous,
          speaker_labels: { ...previous.speaker_labels, [speakerId]: label },
        });
      }
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(queryKey, context.previous);
      }
    },
    onSuccess: (updated) => {
      queryClient.setQueryData(queryKey, updated);
    },
  });

  const onRename = useCallback(
    (speakerId: number, label: string) => rename.mutate({ speakerId, label }),
    [rename],
  );

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <Link
          to="/"
          className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-4" />
          Conversations
        </Link>
        <LiveBadge connected={connected} />
      </div>

      {isLoading && <Skeleton className="h-64 w-full" />}
      {error && (
        <Card>
          <CardContent className="py-6 text-center text-destructive">
            {(error as Error).message}
          </CardContent>
        </Card>
      )}

      {conversation && (
        <>
          <Card>
            <CardHeader className="gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <CardTitle className="font-mono">{conversation.id}</CardTitle>
                {conversation.backend && (
                  <Badge variant="outline">{conversation.backend}</Badge>
                )}
                {conversation.language && (
                  <Badge variant="outline">{conversation.language}</Badge>
                )}
                <Badge variant="outline">
                  {formatDuration(conversation.total_audio_secs)}
                </Badge>
                <Badge>{conversation.turns.length} turns</Badge>
              </div>
              <SpeakerLegend conversation={conversation} onRename={onRename} />
            </CardHeader>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Transcript</CardTitle>
            </CardHeader>
            <CardContent>
              <TranscriptView conversation={conversation} onRename={onRename} />
              {partial && (
                <p className="mt-3 rounded-md border border-dashed p-3 text-sm italic text-muted-foreground">
                  {partial}
                </p>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
