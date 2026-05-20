import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Play, Square, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useToast } from "@/components/ui/toast";
import { api } from "@/lib/api";

interface Props {
  agentId: string;
  running: boolean;
  size?: "sm" | "md";
}

/** Spawn / stop / reload controls for an agent, with toasts + cache busting. */
export function AgentActions({ agentId, running, size = "sm" }: Props) {
  const qc = useQueryClient();
  const { toast } = useToast();

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["agents"] });
    qc.invalidateQueries({ queryKey: ["agent", agentId] });
  };

  const spawn = useMutation({
    mutationFn: () => api.spawnAgent(agentId),
    onSuccess: () => {
      toast(`Spawned ${agentId}`, "success");
      invalidate();
    },
    onError: (e) => toast(String(e instanceof Error ? e.message : e), "error"),
  });

  const stop = useMutation({
    mutationFn: () => api.stopAgent(agentId),
    onSuccess: () => {
      toast(`Stopped ${agentId}`, "success");
      invalidate();
    },
    onError: (e) => toast(String(e instanceof Error ? e.message : e), "error"),
  });

  const reload = useMutation({
    mutationFn: () => api.reloadAgent(agentId),
    onSuccess: () => {
      toast(`Reloaded ${agentId}`, "success");
      invalidate();
    },
    onError: (e) => toast(String(e instanceof Error ? e.message : e), "error"),
  });

  const busy = spawn.isPending || stop.isPending || reload.isPending;

  return (
    <div className="flex items-center gap-1.5">
      {running ? (
        <Button
          size={size}
          variant="destructive"
          disabled={busy}
          onClick={() => stop.mutate()}
        >
          <Square className="size-3.5" /> Stop
        </Button>
      ) : (
        <Button size={size} disabled={busy} onClick={() => spawn.mutate()}>
          <Play className="size-3.5" /> Spawn
        </Button>
      )}
      <Button
        size={size}
        variant="outline"
        disabled={busy}
        onClick={() => reload.mutate()}
      >
        <RotateCw className="size-3.5" /> Reload
      </Button>
    </div>
  );
}
