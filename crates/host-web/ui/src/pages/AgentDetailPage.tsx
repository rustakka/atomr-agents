import { useState } from "react";
import { useParams, Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { api } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Tabs, type TabItem } from "@/components/ui/tabs";
import { ErrorState, SkeletonRows } from "@/components/ui/states";
import { AgentActions } from "@/components/agent/AgentActions";
import { DocTab } from "@/pages/tabs/DocTab";
import { SkillsTab } from "@/pages/tabs/SkillsTab";
import { HooksTab } from "@/pages/tabs/HooksTab";
import { BranchesTab } from "@/pages/tabs/BranchesTab";
import { EvalsTab } from "@/pages/tabs/EvalsTab";
import { ChatTab } from "@/pages/tabs/ChatTab";

const TABS: TabItem[] = [
  { value: "soul", label: "Identity (SOUL)" },
  { value: "rules", label: "Rules" },
  { value: "memory", label: "Memory" },
  { value: "user", label: "User" },
  { value: "skills", label: "Skills" },
  { value: "hooks", label: "Hooks" },
  { value: "branches", label: "Branches" },
  { value: "evals", label: "Evals" },
  { value: "chat", label: "Chat" },
];

export default function AgentDetailPage() {
  const { id = "" } = useParams();
  const [tab, setTab] = useState("soul");

  const agent = useQuery({
    queryKey: ["agent", id],
    queryFn: () => api.getAgent(id),
    enabled: !!id,
  });

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-4">
      <Link
        to="/agents"
        className="flex w-fit items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="size-3.5" /> Agents
      </Link>

      {agent.isLoading && <SkeletonRows rows={2} />}
      {agent.error && <ErrorState error={agent.error} />}

      {agent.data && (
        <header className="flex flex-wrap items-center gap-3 border-b pb-4">
          <div className="min-w-0">
            <h1 className="truncate text-lg font-semibold">{agent.data.id}</h1>
            <p className="text-xs text-muted-foreground">{agent.data.model}</p>
          </div>
          <Badge variant={agent.data.running ? "success" : "outline"}>
            {agent.data.running ? "running" : "stopped"}
          </Badge>
          <div className="ml-auto">
            <AgentActions agentId={agent.data.id} running={agent.data.running} size="md" />
          </div>
        </header>
      )}

      <Tabs tabs={TABS} value={tab} onValueChange={setTab} />

      <div>
        {id && tab === "soul" && <DocTab agentId={id} doc="soul" />}
        {id && tab === "rules" && <DocTab agentId={id} doc="rules" />}
        {id && tab === "memory" && <DocTab agentId={id} doc="memory" />}
        {id && tab === "user" && <DocTab agentId={id} doc="user" />}
        {id && tab === "skills" && <SkillsTab agentId={id} />}
        {id && tab === "hooks" && <HooksTab agentId={id} />}
        {id && tab === "branches" && <BranchesTab agentId={id} />}
        {id && tab === "evals" && <EvalsTab agentId={id} />}
        {id && tab === "chat" && <ChatTab agentId={id} />}
      </div>
    </div>
  );
}
