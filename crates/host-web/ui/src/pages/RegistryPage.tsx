import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Trash2, Eye } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { Dialog } from "@/components/ui/dialog";
import { JsonView } from "@/components/ui/json-view";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { useToast } from "@/components/ui/toast";
import { formatRelativeMs } from "@/lib/utils";
import type { CachedArtifact } from "@/lib/apiTypes";

export default function RegistryPage() {
  const qc = useQueryClient();
  const { toast } = useToast();
  const [kind, setKind] = useState("");
  const [viewing, setViewing] = useState<CachedArtifact | null>(null);

  const registry = useQuery({
    queryKey: ["registry", kind],
    queryFn: () => api.listRegistry(kind.trim() || undefined),
  });

  const remove = useMutation({
    mutationFn: (a: CachedArtifact) =>
      api.deleteArtifact(a.kind, a.id, a.version),
    onSuccess: () => {
      toast("Artifact deleted", "success");
      qc.invalidateQueries({ queryKey: ["registry"] });
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <h1 className="text-lg font-semibold">Registry</h1>
        <Input
          value={kind}
          placeholder="filter by kind…"
          className="ml-auto w-48"
          onChange={(e) => setKind(e.target.value)}
        />
      </div>

      {registry.isLoading && <SkeletonRows rows={4} />}
      {registry.error && <ErrorState error={registry.error} />}
      {registry.data && registry.data.artifacts.length === 0 && (
        <EmptyState title="No cached artifacts" />
      )}
      {registry.data && registry.data.artifacts.length > 0 && (
        <Card>
          <CardContent className="pt-4">
            <Table>
              <THead>
                <Tr>
                  <Th>Kind</Th>
                  <Th>ID</Th>
                  <Th>Version</Th>
                  <Th>Cached</Th>
                  <Th className="text-right">Actions</Th>
                </Tr>
              </THead>
              <TBody>
                {registry.data.artifacts.map((a) => (
                  <Tr key={`${a.kind}/${a.id}/${a.version}`}>
                    <Td>
                      <Badge variant="outline">{a.kind}</Badge>
                    </Td>
                    <Td className="font-medium">{a.id}</Td>
                    <Td className="text-muted-foreground">{a.version}</Td>
                    <Td className="text-muted-foreground">
                      {formatRelativeMs(a.cached_at_ms)}
                    </Td>
                    <Td>
                      <div className="flex justify-end gap-1.5">
                        <Button size="sm" variant="outline" onClick={() => setViewing(a)}>
                          <Eye className="size-3.5" /> View
                        </Button>
                        <Button
                          size="sm"
                          variant="destructive"
                          disabled={remove.isPending}
                          onClick={() => remove.mutate(a)}
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

      <Dialog
        open={!!viewing}
        onClose={() => setViewing(null)}
        title={viewing ? `${viewing.kind} / ${viewing.id} @ ${viewing.version}` : ""}
      >
        {viewing && <JsonView value={viewing.payload} />}
      </Dialog>
    </div>
  );
}
