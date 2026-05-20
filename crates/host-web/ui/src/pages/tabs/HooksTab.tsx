import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { prettyJson } from "@/lib/utils";

export function HooksTab({ agentId }: { agentId: string }) {
  const hooks = useQuery({
    queryKey: ["agent", agentId, "hooks"],
    queryFn: () => api.listHooks(agentId),
  });

  if (hooks.isLoading) return <SkeletonRows rows={3} />;
  if (hooks.error) return <ErrorState error={hooks.error} />;
  if (!hooks.data || hooks.data.hooks.length === 0)
    return <EmptyState title="No hooks configured" />;

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Event</Th>
          <Th>When</Th>
          <Th>Match</Th>
          <Th>Call</Th>
        </Tr>
      </THead>
      <TBody>
        {hooks.data.hooks.map((hook, i) => (
          <Tr key={`${hook.event}-${i}`}>
            <Td>
              <Badge variant="outline">{hook.event}</Badge>
            </Td>
            <Td className="text-muted-foreground">{hook.when}</Td>
            <Td>
              <pre className="max-w-xs overflow-auto text-xs text-muted-foreground">
                {prettyJson(hook.match)}
              </pre>
            </Td>
            <Td>
              <pre className="max-w-xs overflow-auto text-xs text-muted-foreground">
                {prettyJson(hook.call)}
              </pre>
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
