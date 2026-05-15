import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { api } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { MeetingList } from "@/components/meeting/MeetingList";

/** Landing: list of meetings + dialog to trigger a new analysis. */
export default function MeetingsPage() {
  return (
    <div className="flex flex-col gap-4">
      <NewAnalysisCard />
      <MeetingList />
    </div>
  );
}

function NewAnalysisCard() {
  const [open, setOpen] = useState(false);
  const [transcriptId, setTranscriptId] = useState("");
  const [modelId, setModelId] = useState("");
  const [mode, setMode] = useState<"batch" | "live">("batch");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const { data: transcripts = [] } = useQuery({
    queryKey: ["transcripts"],
    queryFn: api.listTranscripts,
    enabled: open,
  });

  const trigger = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.triggerRun(transcriptId, { mode, model_id: modelId });
      setOpen(false);
      setTranscriptId("");
      setModelId("");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <Card>
        <CardContent className="flex items-center justify-between py-3">
          <p className="text-sm text-muted-foreground">
            Trigger a meetings analysis over an existing STT transcript.
          </p>
          <Button onClick={() => setOpen(true)}>
            <Play className="size-4" />
            New analysis
          </Button>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>New analysis</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Source transcript id</span>
          <Input
            value={transcriptId}
            onChange={(e) => setTranscriptId(e.target.value)}
            placeholder="conversation_id"
            list="transcripts"
          />
          <datalist id="transcripts">
            {transcripts.map((t) => (
              <option key={t.id} value={t.id} />
            ))}
          </datalist>
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Model id</span>
          <Input
            value={modelId}
            onChange={(e) => setModelId(e.target.value)}
            placeholder="e.g. claude-opus-4-7"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Mode</span>
          <select
            value={mode}
            onChange={(e) => setMode(e.target.value as "batch" | "live")}
            className="h-9 rounded-md border border-input bg-background px-3 text-sm"
          >
            <option value="batch">batch</option>
            <option value="live">live (CLI only for now)</option>
          </select>
        </label>
        {error && (
          <Badge variant="destructive" className="self-start">
            {error}
          </Badge>
        )}
        <div className="flex gap-2">
          <Button
            onClick={trigger}
            disabled={busy || !transcriptId || !modelId}
          >
            {busy ? "running…" : "Run"}
          </Button>
          <Button variant="outline" onClick={() => setOpen(false)} disabled={busy}>
            Cancel
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
