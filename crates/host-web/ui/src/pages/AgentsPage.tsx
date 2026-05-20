import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { api } from "@/lib/api";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { AgentActions } from "@/components/agent/AgentActions";

export default function AgentsPage() {
  const agents = useQuery({ queryKey: ["agents"], queryFn: api.listAgents });

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-4">
      <h1 className="text-lg font-semibold">Agents</h1>

      {agents.isLoading && <SkeletonRows rows={4} />}
      {agents.error && <ErrorState error={agents.error} />}
      {agents.data && agents.data.agents.length === 0 && (
        <EmptyState
          title="No agents yet"
          hint={
            <>
              Run <code className="rounded bg-muted px-1">atomr-host init</code> to scaffold one.
            </>
          }
        />
      )}
      {agents.data && agents.data.agents.length > 0 && (
        <Card>
          <CardContent className="pt-4">
            <Table>
              <THead>
                <Tr>
                  <Th>Agent</Th>
                  <Th>Model</Th>
                  <Th>State</Th>
                  <Th className="text-right">Rules</Th>
                  <Th className="text-right">Memory</Th>
                  <Th className="text-right">Skills</Th>
                  <Th className="text-right">Actions</Th>
                </Tr>
              </THead>
              <TBody>
                {agents.data.agents.map((agent) => (
                  <Tr key={agent.id}>
                    <Td>
                      <Link
                        to={`/agents/${encodeURIComponent(agent.id)}`}
                        className="font-medium text-primary hover:underline"
                      >
                        {agent.id}
                      </Link>
                    </Td>
                    <Td className="text-muted-foreground">{agent.model}</Td>
                    <Td>
                      <Badge variant={agent.running ? "success" : "outline"}>
                        {agent.running ? "running" : "stopped"}
                      </Badge>
                    </Td>
                    <Td className="text-right">{agent.rules_count}</Td>
                    <Td className="text-right">{agent.memory_facts_count}</Td>
                    <Td className="text-right">{agent.skills_count}</Td>
                    <Td>
                      <div className="flex justify-end">
                        <AgentActions agentId={agent.id} running={agent.running} />
                      </div>
                    </Td>
                  </Tr>
                ))}
              </TBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
