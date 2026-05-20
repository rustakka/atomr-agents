import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { DocEditor, type DocEditorValue } from "@/components/agent/DocEditor";
import { ErrorState, SkeletonRows } from "@/components/ui/states";
import { useToast } from "@/components/ui/toast";
import type { DocKind } from "@/lib/apiTypes";

export function DocTab({ agentId, doc }: { agentId: string; doc: DocKind }) {
  const qc = useQueryClient();
  const { toast } = useToast();

  const query = useQuery({
    queryKey: ["agent", agentId, "doc", doc],
    queryFn: () => api.getDoc(agentId, doc),
  });

  const save = useMutation({
    mutationFn: (value: DocEditorValue) => api.putDoc(agentId, doc, value),
    onSuccess: () => {
      toast(`Saved ${doc} doc`, "success");
      qc.invalidateQueries({ queryKey: ["agent", agentId, "doc", doc] });
      qc.invalidateQueries({ queryKey: ["agent", agentId] });
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  if (query.isLoading) return <SkeletonRows rows={3} />;
  if (query.error) return <ErrorState error={query.error} />;
  if (!query.data) return null;

  return (
    <DocEditor
      frontmatter={query.data.frontmatter}
      body={query.data.body}
      sourcePath={query.data.source_path}
      saving={save.isPending}
      onSave={(value) => save.mutate(value)}
    />
  );
}
