// TS interfaces mirroring the `atomr-agents-host` axum serde types.
// Keep in sync with the host-web backend handlers.

export interface AgentSummary {
  id: string;
  model: string;
  persona_identity: string | null;
  running: boolean;
  rules_count: number;
  memory_facts_count: number;
  skills_count: number;
  user_profile_len: number;
}

export interface DocDto {
  frontmatter: Record<string, unknown>;
  body: string;
  source_path?: string | null;
}

export type DocKind = "soul" | "rules" | "memory" | "user";

export interface SkillDef {
  id: string;
  name: string;
  instruction_fragment: string;
  priority: number;
  keywords: string[];
  tool_overlay: string[];
  memory_namespace: string[];
  source_path?: string | null;
}

export interface HookDef {
  event: string;
  match: Record<string, unknown>;
  call: Record<string, unknown>;
  when: string;
  budget: Record<string, unknown>;
  source_path?: string | null;
}

export interface AgentIdentity {
  agent_id: string;
  model: string;
  persona_identity: string | null;
}

export interface AgentDocs {
  soul: DocDto;
  rules: DocDto;
  memory: DocDto;
  user: DocDto;
}

export interface AgentDetail {
  id: string;
  model: string;
  running: boolean;
  spec: Record<string, unknown>;
  identity: AgentIdentity | null;
  status: AgentSummary | null;
  docs: AgentDocs;
  skills: SkillDef[];
  hooks: HookDef[];
}

export interface CronEntry {
  id: string;
  expression: string;
  call: unknown;
  input: unknown;
  enabled: boolean;
}

export interface Checkpoint {
  branch_id: string;
  agent_id: string;
  ts_ms: number;
  working_memory: Record<string, unknown>;
  thread_head?: string | null;
  parent_branch?: string | null;
}

export interface BranchDiff {
  added_keys: string[];
  removed_keys: string[];
  changed_keys: { key: string; a: unknown; b: unknown }[];
}

export interface CachedArtifact {
  kind: string;
  id: string;
  version: string;
  payload: unknown;
  cached_at_ms: number;
  path?: string | null;
}

export interface EvalCase {
  id: string;
  input: unknown;
  expected: unknown;
}

export interface EvalSuite {
  id: string;
  scorer: string;
  description?: string | null;
  cases: EvalCase[];
}

export interface EvalRunResult {
  case_id: string;
  passed: boolean;
  score: number;
  reason?: string | null;
  output: unknown;
}

export interface EvalRun {
  suite_id: string;
  agent_id: string;
  results: EvalRunResult[];
  passed: number;
  total: number;
}

export interface McpTool {
  name: string;
  description: string;
  schema: unknown;
}

export interface McpServer {
  id: string;
  command: string[];
  env: Record<string, string>;
  tools: McpTool[];
}

export interface EventRecord {
  ts_ms: number;
  kind: string;
  agent_id?: string | null;
  payload: unknown;
}

export interface Concept {
  key: string;
  label: string;
  primitive: string;
  borrowed_from: string;
  api: string;
  ui_section: string;
  description: string;
}

export interface SkillReport {
  skill_id: string;
  path: string;
  errors: string[];
  warnings: string[];
}

export interface RoutesDto {
  default_agent: string | null;
  channel_pins: Record<string, string>;
  peer_pins: Record<string, string>;
}

export interface ConfigDto {
  yaml: string;
  parsed: Record<string, unknown>;
}

export interface BranchesDto {
  current: string;
  branches: string[];
}

export interface OkResponse {
  ok: boolean;
  path?: string;
}
