import { useState } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { GitBranch, Trash2, GitCompare } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { JsonView } from "@/components/ui/json-view";
import { useToast } from "@/components/ui/toast";
import { cn } from "@/lib/utils";
import type { BranchDiff } from "@/lib/apiTypes";

export function BranchesTab({ agentId }: { agentId: string }) {
  const qc = useQueryClient();
  const { toast } = useToast();
  const [newName, setNewName] = useState("");
  const [source, setSource] = useState("");
  const [diffA, setDiffA] = useState("");
  const [diffB, setDiffB] = useState("");
  const [diff, setDiff] = useState<BranchDiff | null>(null);

  const branches = useQuery({
    queryKey: ["agent", agentId, "branches"],
    queryFn: () => api.listBranches(agentId),
  });

  const invalidate = () =>
    qc.invalidateQueries({ queryKey: ["agent", agentId, "branches"] });

  const create = useMutation({
    mutationFn: () =>
      api.createBranch(agentId, {
        new: newName.trim(),
        source: source.trim() || undefined,
      }),
    onSuccess: () => {
      toast(`Created branch ${newName}`, "success");
      setNewName("");
      setSource("");
      invalidate();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  const switchTo = useMutation({
    mutationFn: (b: string) => api.switchBranch(agentId, b),
    onSuccess: (_d, b) => {
      toast(`Switched to ${b}`, "success");
      invalidate();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  const remove = useMutation({
    mutationFn: (b: string) => api.deleteBranch(agentId, b),
    onSuccess: (_d, b) => {
      toast(`Deleted ${b}`, "success");
      invalidate();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  const computeDiff = useMutation({
    mutationFn: () => api.branchDiff(agentId, diffA.trim(), diffB.trim()),
    onSuccess: (d) => setDiff(d),
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  if (branches.isLoading) return <SkeletonRows rows={3} />;
  if (branches.error) return <ErrorState error={branches.error} />;

  const current = branches.data?.current;
  const list = branches.data?.branches ?? [];

  return (
    <div className="flex flex-col gap-5">
      <Card>
        <CardHeader>
          <CardTitle>Branches</CardTitle>
        </CardHeader>
        <CardContent>
          {list.length === 0 ? (
            <EmptyState title="No branches" />
          ) : (
            <ul className="flex flex-col gap-1.5">
              {list.map((b) => (
                <li
                  key={b}
                  className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
                >
                  <GitBranch className="size-4 text-muted-foreground" />
                  <span className={cn(b === current && "font-semibold")}>{b}</span>
                  {b === current && <Badge variant="success">current</Badge>}
                  <div className="ml-auto flex gap-1.5">
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={b === current || switchTo.isPending}
                      onClick={() => switchTo.mutate(b)}
                    >
                      Switch
                    </Button>
                    <Button
                      size="sm"
                      variant="destructive"
                      disabled={b === current || remove.isPending}
                      onClick={() => remove.mutate(b)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Fork a branch</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap items-end gap-2">
          <div className="flex flex-col gap-1">
            <label className="text-xs text-muted-foreground">Source (optional)</label>
            <Input
              value={source}
              placeholder={current ?? "current"}
              onChange={(e) => setSource(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-xs text-muted-foreground">New name</label>
            <Input
              value={newName}
              placeholder="experiment"
              onChange={(e) => setNewName(e.target.value)}
            />
          </div>
          <Button
            disabled={!newName.trim() || create.isPending}
            onClick={() => create.mutate()}
          >
            Create
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Diff branches</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex flex-wrap items-end gap-2">
            <div className="flex flex-col gap-1">
              <label className="text-xs text-muted-foreground">Branch A</label>
              <Input value={diffA} onChange={(e) => setDiffA(e.target.value)} />
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs text-muted-foreground">Branch B</label>
              <Input value={diffB} onChange={(e) => setDiffB(e.target.value)} />
            </div>
            <Button
              variant="outline"
              disabled={!diffA.trim() || !diffB.trim() || computeDiff.isPending}
              onClick={() => computeDiff.mutate()}
            >
              <GitCompare className="size-3.5" /> Compare
            </Button>
          </div>

          {diff && (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
              <DiffColumn title="Added" variant="success" keys={diff.added_keys} />
              <DiffColumn title="Removed" variant="destructive" keys={diff.removed_keys} />
              <div>
                <p className="mb-1 text-xs font-medium text-muted-foreground">
                  Changed ({diff.changed_keys.length})
                </p>
                <div className="flex flex-col gap-2">
                  {diff.changed_keys.length === 0 ? (
                    <p className="text-xs text-muted-foreground">—</p>
                  ) : (
                    diff.changed_keys.map((c) => (
                      <div key={c.key} className="rounded-md border p-2">
                        <p className="text-xs font-medium">{c.key}</p>
                        <div className="mt-1 grid grid-cols-2 gap-1">
                          <JsonView value={c.a} className="max-h-32" />
                          <JsonView value={c.b} className="max-h-32" />
                        </div>
                      </div>
                    ))
                  )}
                </div>
              </div>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function DiffColumn({
  title,
  variant,
  keys,
}: {
  title: string;
  variant: "success" | "destructive";
  keys: string[];
}) {
  return (
    <div>
      <p className="mb-1 text-xs font-medium text-muted-foreground">
        {title} ({keys.length})
      </p>
      <div className="flex flex-col gap-1">
        {keys.length === 0 ? (
          <p className="text-xs text-muted-foreground">—</p>
        ) : (
          keys.map((k) => (
            <Badge key={k} variant={variant} className="w-fit">
              {k}
            </Badge>
          ))
        )}
      </div>
    </div>
  );
}
