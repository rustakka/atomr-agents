import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { Brain, ListChecks, Sparkles } from "lucide-react";
import { api } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { EventTicker } from "@/components/agent/EventTicker";

export default function OverviewPage() {
  const agents = useQuery({ queryKey: ["agents"], queryFn: api.listAgents });

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-6">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold">Overview</h1>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[1fr_20rem]">
        <section className="flex flex-col gap-3">
          <h2 className="text-sm font-medium text-muted-foreground">Agents</h2>
          {agents.isLoading && <SkeletonRows rows={3} />}
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
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
              {agents.data.agents.map((agent) => (
                <Link key={agent.id} to={`/agents/${encodeURIComponent(agent.id)}`}>
                  <Card className="h-full transition-colors hover:border-primary/50">
                    <CardHeader className="flex-row items-start justify-between space-y-0">
                      <div className="min-w-0">
                        <CardTitle className="truncate">{agent.id}</CardTitle>
                        <p className="mt-1 truncate text-xs text-muted-foreground">
                          {agent.model}
                        </p>
                      </div>
                      <Badge variant={agent.running ? "success" : "outline"}>
                        {agent.running ? "running" : "stopped"}
                      </Badge>
                    </CardHeader>
                    <CardContent className="flex flex-col gap-3">
                      {agent.persona_identity && (
                        <p className="line-clamp-2 text-xs text-muted-foreground">
                          {agent.persona_identity}
                        </p>
                      )}
                      <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
                        <span className="flex items-center gap-1">
                          <ListChecks className="size-3.5" /> {agent.rules_count} rules
                        </span>
                        <span className="flex items-center gap-1">
                          <Brain className="size-3.5" /> {agent.memory_facts_count} memory
                        </span>
                        <span className="flex items-center gap-1">
                          <Sparkles className="size-3.5" /> {agent.skills_count} skills
                        </span>
                      </div>
                    </CardContent>
                  </Card>
                </Link>
              ))}
            </div>
          )}
        </section>

        <aside>
          <Card>
            <CardContent className="pt-4">
              <EventTicker limit={10} />
            </CardContent>
          </Card>
        </aside>
      </div>
    </div>
  );
}
