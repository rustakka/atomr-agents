import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { useToast } from "@/components/ui/toast";

export default function McpPage() {
  const qc = useQueryClient();
  const { toast } = useToast();
  const [id, setId] = useState("");
  const [command, setCommand] = useState("");

  const servers = useQuery({ queryKey: ["mcp"], queryFn: api.listMcp });

  const create = useMutation({
    mutationFn: () =>
      api.createMcp({
        id: id.trim(),
        command: command
          .split(/\s+/)
          .map((c) => c.trim())
          .filter(Boolean),
      }),
    onSuccess: () => {
      toast(`Added MCP server ${id}`, "success");
      setId("");
      setCommand("");
      qc.invalidateQueries({ queryKey: ["mcp"] });
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4">
      <h1 className="text-lg font-semibold">MCP Servers</h1>

      {servers.isLoading && <SkeletonRows rows={3} />}
      {servers.error && <ErrorState error={servers.error} />}
      {servers.data && servers.data.servers.length === 0 && (
        <EmptyState title="No MCP servers configured" />
      )}
      {servers.data && servers.data.servers.length > 0 && (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          {servers.data.servers.map((s) => (
            <Card key={s.id}>
              <CardHeader>
                <CardTitle>{s.id}</CardTitle>
                <p className="font-mono text-xs text-muted-foreground">
                  {s.command.join(" ")}
                </p>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                <p className="text-xs text-muted-foreground">
                  {s.tools.length} tool{s.tools.length === 1 ? "" : "s"}
                </p>
                <div className="flex flex-wrap gap-1">
                  {s.tools.map((t) => (
                    <Badge key={t.name} variant="outline" title={t.description}>
                      {t.name}
                    </Badge>
                  ))}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Add server</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap items-end gap-2">
          <label className="flex flex-col gap-1">
            <span className="text-xs text-muted-foreground">ID</span>
            <Input value={id} onChange={(e) => setId(e.target.value)} placeholder="filesystem" />
          </label>
          <label className="flex flex-1 flex-col gap-1">
            <span className="text-xs text-muted-foreground">
              Command (space-separated)
            </span>
            <Input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder="npx -y @modelcontextprotocol/server-filesystem ."
            />
          </label>
          <Button
            disabled={!id.trim() || !command.trim() || create.isPending}
            onClick={() => create.mutate()}
          >
            <Plus className="size-3.5" /> Add
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
