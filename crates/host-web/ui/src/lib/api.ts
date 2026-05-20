// Typed REST client for the `atomr-agents-host` axum backend. All routes are
// served under `/api` on the same origin (dev-proxied to 127.0.0.1:7400).

import type {
  AgentDetail,
  AgentSummary,
  BranchDiff,
  BranchesDto,
  CachedArtifact,
  Checkpoint,
  Concept,
  ConfigDto,
  CronEntry,
  DocDto,
  DocKind,
  EvalRun,
  EvalSuite,
  EventRecord,
  HookDef,
  McpServer,
  OkResponse,
  RoutesDto,
  SkillDef,
  SkillReport,
} from "./apiTypes";

const enc = encodeURIComponent;

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, { credentials: "same-origin", ...init });
  if (!resp.ok) {
    let detail = `${resp.status} ${resp.statusText}`;
    try {
      const body = await resp.json();
      if (body?.error) detail = body.error;
      else if (typeof body === "string") detail = body;
    } catch {
      // non-JSON error body
    }
    throw new Error(detail);
  }
  if (resp.status === 204) return undefined as T;
  return resp.json() as Promise<T>;
}

function jsonBody(method: string, body: unknown): RequestInit {
  return {
    method,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  };
}

export const api = {
  // --- concepts -----------------------------------------------------------
  listConcepts: () => request<{ concepts: Concept[] }>("/api/concepts"),

  // --- agents -------------------------------------------------------------
  listAgents: () => request<{ agents: AgentSummary[] }>("/api/agents"),

  getAgent: (id: string) => request<AgentDetail>(`/api/agents/${enc(id)}`),

  spawnAgent: (id: string) =>
    request<OkResponse>(`/api/agents/${enc(id)}/spawn`, { method: "POST" }),

  stopAgent: (id: string) =>
    request<void>(`/api/agents/${enc(id)}`, { method: "DELETE" }),

  reloadAgent: (id: string) =>
    request<OkResponse>(`/api/agents/${enc(id)}/reload`, { method: "POST" }),

  chat: (id: string, message: string) =>
    request<{ reply: string }>(
      `/api/agents/${enc(id)}/chat`,
      jsonBody("POST", { message }),
    ),

  // --- docs ---------------------------------------------------------------
  getDoc: (id: string, doc: DocKind) =>
    request<DocDto>(`/api/agents/${enc(id)}/docs/${enc(doc)}`),

  putDoc: (
    id: string,
    doc: DocKind,
    body: { frontmatter: Record<string, unknown>; body: string },
  ) =>
    request<OkResponse>(
      `/api/agents/${enc(id)}/docs/${enc(doc)}`,
      jsonBody("PUT", body),
    ),

  // --- skills -------------------------------------------------------------
  listSkills: (id: string) =>
    request<{ skills: SkillDef[] }>(`/api/agents/${enc(id)}/skills`),

  createSkill: (
    id: string,
    body: { id: string; name: string; priority?: number; keywords?: string[] },
  ) => request<OkResponse>(`/api/agents/${enc(id)}/skills`, jsonBody("POST", body)),

  putSkill: (
    id: string,
    sid: string,
    body: { frontmatter: Record<string, unknown>; body: string },
  ) =>
    request<OkResponse>(
      `/api/agents/${enc(id)}/skills/${enc(sid)}`,
      jsonBody("PUT", body),
    ),

  deleteSkill: (id: string, sid: string) =>
    request<void>(`/api/agents/${enc(id)}/skills/${enc(sid)}`, {
      method: "DELETE",
    }),

  validateSkills: (id: string) =>
    request<{ reports: SkillReport[] }>(
      `/api/agents/${enc(id)}/skills/validate`,
    ),

  // --- curator ------------------------------------------------------------
  listProposals: (id: string) =>
    request<{ proposals: string[] }>(`/api/agents/${enc(id)}/curator/proposals`),

  approveProposal: (id: string, sid: string) =>
    request<OkResponse>(
      `/api/agents/${enc(id)}/curator/proposals/${enc(sid)}/approve`,
      { method: "POST" },
    ),

  rejectProposal: (id: string, sid: string) =>
    request<OkResponse>(
      `/api/agents/${enc(id)}/curator/proposals/${enc(sid)}/reject`,
      { method: "POST" },
    ),

  skillHistory: (id: string, sid: string) =>
    request<{ history: string[] }>(
      `/api/agents/${enc(id)}/curator/history/${enc(sid)}`,
    ),

  revertSkill: (id: string, sid: string) =>
    request<OkResponse>(`/api/agents/${enc(id)}/curator/revert/${enc(sid)}`, {
      method: "POST",
    }),

  // --- hooks --------------------------------------------------------------
  listHooks: (id: string) =>
    request<{ hooks: HookDef[] }>(`/api/agents/${enc(id)}/hooks`),

  // --- branches -----------------------------------------------------------
  listBranches: (id: string) =>
    request<BranchesDto>(`/api/agents/${enc(id)}/branches`),

  createBranch: (id: string, body: { source?: string; new: string }) =>
    request<{ ok: boolean; checkpoint: Checkpoint }>(
      `/api/agents/${enc(id)}/branches`,
      jsonBody("POST", body),
    ),

  switchBranch: (id: string, branch: string) =>
    request<OkResponse>(
      `/api/agents/${enc(id)}/branches/${enc(branch)}/switch`,
      { method: "POST" },
    ),

  branchDiff: (id: string, a: string, b: string) =>
    request<BranchDiff>(
      `/api/agents/${enc(id)}/branches/diff?a=${enc(a)}&b=${enc(b)}`,
    ),

  deleteBranch: (id: string, branch: string) =>
    request<void>(
      `/api/agents/${enc(id)}/branches/${enc(branch)}?force=true`,
      { method: "DELETE" },
    ),

  // --- evals (host-wide) --------------------------------------------------
  listEvals: () => request<{ suites: string[] }>("/api/evals"),

  getEvalSuite: (suiteId: string) =>
    request<EvalSuite>(`/api/evals/${enc(suiteId)}`),

  runEval: (suiteId: string, agentId: string) =>
    request<EvalRun>(`/api/evals/${enc(suiteId)}/run?agent=${enc(agentId)}`, {
      method: "POST",
    }),

  // --- crons --------------------------------------------------------------
  listCrons: () => request<{ crons: CronEntry[] }>("/api/crons"),

  createCron: (body: { id: string; expression: string; call: unknown }) =>
    request<OkResponse>("/api/crons", jsonBody("POST", body)),

  deleteCron: (id: string) =>
    request<void>(`/api/crons/${enc(id)}`, { method: "DELETE" }),

  // --- routes / channels --------------------------------------------------
  getRoutes: () => request<RoutesDto>("/api/routes"),

  listChannels: () => request<{ channels: string[] }>("/api/channels"),

  // --- registry -----------------------------------------------------------
  listRegistry: (kind?: string) =>
    request<{ artifacts: CachedArtifact[] }>(
      kind ? `/api/registry?kind=${enc(kind)}` : "/api/registry",
    ),

  deleteArtifact: (kind: string, id: string, version: string) =>
    request<void>(
      `/api/registry/${enc(kind)}/${enc(id)}/${enc(version)}`,
      { method: "DELETE" },
    ),

  // --- mcp ----------------------------------------------------------------
  listMcp: () => request<{ servers: McpServer[] }>("/api/mcp"),

  createMcp: (body: { id: string; command: string[] }) =>
    request<OkResponse>("/api/mcp", jsonBody("POST", body)),

  // --- config -------------------------------------------------------------
  getConfig: () => request<ConfigDto>("/api/config"),

  putConfig: (yaml: string) =>
    request<OkResponse>("/api/config", jsonBody("PUT", { yaml })),

  // --- events -------------------------------------------------------------
  listEvents: (limit = 200) =>
    request<{ events: EventRecord[] }>(`/api/events?limit=${limit}`),
};
