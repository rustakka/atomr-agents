import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { useToast } from "@/components/ui/toast";
import { prettyJson } from "@/lib/utils";

export default function CronsPage() {
  const qc = useQueryClient();
  const { toast } = useToast();
  const [id, setId] = useState("");
  const [expression, setExpression] = useState("every:5m");
  const [callJson, setCallJson] = useState('{\n  "tool": "noop"\n}');

  const crons = useQuery({ queryKey: ["crons"], queryFn: api.listCrons });

  const invalidate = () => qc.invalidateQueries({ queryKey: ["crons"] });

  const create = useMutation({
    mutationFn: () => {
      let call: unknown;
      try {
        call = JSON.parse(callJson);
      } catch {
        throw new Error("Call must be valid JSON.");
      }
      return api.createCron({ id: id.trim(), expression: expression.trim(), call });
    },
    onSuccess: () => {
      toast(`Created cron ${id}`, "success");
      setId("");
      invalidate();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  const remove = useMutation({
    mutationFn: (cid: string) => api.deleteCron(cid),
    onSuccess: () => {
      toast("Cron deleted", "success");
      invalidate();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-4">
      <h1 className="text-lg font-semibold">Crons</h1>

      {crons.isLoading && <SkeletonRows rows={3} />}
      {crons.error && <ErrorState error={crons.error} />}
      {crons.data && crons.data.crons.length === 0 && (
        <EmptyState title="No crons scheduled" />
      )}
      {crons.data && crons.data.crons.length > 0 && (
        <Card>
          <CardContent className="pt-4">
            <Table>
              <THead>
                <Tr>
                  <Th>ID</Th>
                  <Th>Expression</Th>
                  <Th>Enabled</Th>
                  <Th>Call</Th>
                  <Th className="text-right">Actions</Th>
                </Tr>
              </THead>
              <TBody>
                {crons.data.crons.map((c) => (
                  <Tr key={c.id}>
                    <Td className="font-medium">{c.id}</Td>
                    <Td>
                      <Badge variant="outline">{c.expression}</Badge>
                    </Td>
                    <Td>
                      <Badge variant={c.enabled ? "success" : "outline"}>
                        {c.enabled ? "on" : "off"}
                      </Badge>
                    </Td>
                    <Td>
                      <pre className="max-w-xs overflow-auto text-xs text-muted-foreground">
                        {prettyJson(c.call)}
                      </pre>
                    </Td>
                    <Td>
                      <div className="flex justify-end">
                        <Button
                          size="sm"
                          variant="destructive"
                          disabled={remove.isPending}
                          onClick={() => remove.mutate(c.id)}
                        >
                          <Trash2 className="size-3.5" />
                        </Button>
                      </div>
                    </Td>
                  </Tr>
                ))}
              </TBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>New cron</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex flex-wrap gap-2">
            <label className="flex flex-1 flex-col gap-1">
              <span className="text-xs text-muted-foreground">ID</span>
              <Input value={id} onChange={(e) => setId(e.target.value)} placeholder="nightly" />
            </label>
            <label className="flex flex-1 flex-col gap-1">
              <span className="text-xs text-muted-foreground">Expression</span>
              <Input
                value={expression}
                onChange={(e) => setExpression(e.target.value)}
                placeholder="every:5m"
              />
            </label>
          </div>
          <label className="flex flex-col gap-1">
            <span className="text-xs text-muted-foreground">Call (JSON)</span>
            <Textarea
              value={callJson}
              spellCheck={false}
              onChange={(e) => setCallJson(e.target.value)}
            />
          </label>
          <div className="flex justify-end">
            <Button
              disabled={!id.trim() || !expression.trim() || create.isPending}
              onClick={() => create.mutate()}
            >
              <Plus className="size-3.5" /> Create
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
