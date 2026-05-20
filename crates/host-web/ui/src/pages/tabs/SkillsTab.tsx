import { useState, type ReactNode } from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  AlertTriangle,
  Check,
  History,
  Pencil,
  Plus,
  ShieldCheck,
  Trash2,
  Undo2,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog } from "@/components/ui/dialog";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { DocEditor, type DocEditorValue } from "@/components/agent/DocEditor";
import { useToast } from "@/components/ui/toast";
import type { SkillDef, SkillReport } from "@/lib/apiTypes";

export function SkillsTab({ agentId }: { agentId: string }) {
  const qc = useQueryClient();
  const { toast } = useToast();
  const [reports, setReports] = useState<SkillReport[] | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [editing, setEditing] = useState<SkillDef | null>(null);
  const [historyFor, setHistoryFor] = useState<string | null>(null);

  const skills = useQuery({
    queryKey: ["agent", agentId, "skills"],
    queryFn: () => api.listSkills(agentId),
  });

  const proposals = useQuery({
    queryKey: ["agent", agentId, "proposals"],
    queryFn: () => api.listProposals(agentId),
  });

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["agent", agentId, "skills"] });
    qc.invalidateQueries({ queryKey: ["agent", agentId, "proposals"] });
    qc.invalidateQueries({ queryKey: ["agent", agentId] });
  };

  const onErr = (e: unknown) =>
    toast(e instanceof Error ? e.message : String(e), "error");

  const validate = useMutation({
    mutationFn: () => api.validateSkills(agentId),
    onSuccess: (res) => {
      setReports(res.reports);
      const probs = res.reports.reduce(
        (n, r) => n + r.errors.length + r.warnings.length,
        0,
      );
      toast(
        probs === 0 ? "All skills valid" : `Validation found ${probs} issue(s)`,
        probs === 0 ? "success" : "info",
      );
    },
    onError: onErr,
  });

  const approve = useMutation({
    mutationFn: (sid: string) => api.approveProposal(agentId, sid),
    onSuccess: () => {
      toast("Proposal approved", "success");
      invalidate();
    },
    onError: onErr,
  });

  const reject = useMutation({
    mutationFn: (sid: string) => api.rejectProposal(agentId, sid),
    onSuccess: () => {
      toast("Proposal rejected", "success");
      invalidate();
    },
    onError: onErr,
  });

  const remove = useMutation({
    mutationFn: (sid: string) => api.deleteSkill(agentId, sid),
    onSuccess: () => {
      toast("Skill deleted", "success");
      invalidate();
    },
    onError: onErr,
  });

  if (skills.isLoading) return <SkeletonRows rows={3} />;
  if (skills.error) return <ErrorState error={skills.error} />;

  const skillList = skills.data?.skills ?? [];
  const proposalList = proposals.data?.proposals ?? [];

  const reportFor = (sid: string) => reports?.find((r) => r.skill_id === sid);

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          disabled={validate.isPending}
          onClick={() => validate.mutate()}
        >
          <ShieldCheck className="size-3.5" /> Validate
        </Button>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="size-3.5" /> New skill
        </Button>
      </div>

      {skillList.length === 0 ? (
        <EmptyState title="No skills yet" hint="Create a skill to get started." />
      ) : (
        <Card>
          <CardContent className="pt-4">
            <ul className="flex flex-col gap-2">
              {skillList.map((skill) => {
                const report = reportFor(skill.id);
                return (
                  <li
                    key={skill.id}
                    className="flex flex-col gap-2 rounded-md border p-3"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{skill.name}</span>
                      <Badge variant="outline">{skill.id}</Badge>
                      <Badge>priority {skill.priority}</Badge>
                      <div className="ml-auto flex gap-1.5">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => setEditing(skill)}
                        >
                          <Pencil className="size-3.5" /> Edit
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => setHistoryFor(skill.id)}
                        >
                          <History className="size-3.5" /> History
                        </Button>
                        <Button
                          size="sm"
                          variant="destructive"
                          disabled={remove.isPending}
                          onClick={() => remove.mutate(skill.id)}
                        >
                          <Trash2 className="size-3.5" />
                        </Button>
                      </div>
                    </div>
                    {skill.keywords.length > 0 && (
                      <div className="flex flex-wrap gap-1">
                        {skill.keywords.map((k) => (
                          <Badge key={k} variant="outline" className="text-[10px]">
                            {k}
                          </Badge>
                        ))}
                      </div>
                    )}
                    {report && (report.errors.length > 0 || report.warnings.length > 0) && (
                      <div className="flex flex-col gap-1 text-xs">
                        {report.errors.map((e, i) => (
                          <span key={`e${i}`} className="flex items-center gap-1 text-destructive">
                            <AlertTriangle className="size-3" /> {e}
                          </span>
                        ))}
                        {report.warnings.map((w, i) => (
                          <span key={`w${i}`} className="flex items-center gap-1 text-amber-500">
                            <AlertTriangle className="size-3" /> {w}
                          </span>
                        ))}
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Curator proposals</CardTitle>
        </CardHeader>
        <CardContent>
          {proposals.isLoading && <SkeletonRows rows={2} />}
          {proposals.error && <ErrorState error={proposals.error} />}
          {proposals.data && proposalList.length === 0 && (
            <p className="text-xs text-muted-foreground">No pending proposals.</p>
          )}
          {proposalList.length > 0 && (
            <ul className="flex flex-col gap-1.5">
              {proposalList.map((sid) => (
                <li
                  key={sid}
                  className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
                >
                  <span className="font-medium">{sid}</span>
                  <div className="ml-auto flex gap-1.5">
                    <Button
                      size="sm"
                      disabled={approve.isPending}
                      onClick={() => approve.mutate(sid)}
                    >
                      <Check className="size-3.5" /> Approve
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={reject.isPending}
                      onClick={() => reject.mutate(sid)}
                    >
                      <X className="size-3.5" /> Reject
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <CreateSkillDialog
        agentId={agentId}
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={invalidate}
      />

      {editing && (
        <EditSkillDialog
          agentId={agentId}
          skill={editing}
          onClose={() => setEditing(null)}
          onSaved={invalidate}
        />
      )}

      {historyFor && (
        <HistoryDialog
          agentId={agentId}
          sid={historyFor}
          onClose={() => setHistoryFor(null)}
          onReverted={invalidate}
        />
      )}
    </div>
  );
}

function CreateSkillDialog({
  agentId,
  open,
  onClose,
  onCreated,
}: {
  agentId: string;
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}) {
  const { toast } = useToast();
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [priority, setPriority] = useState("0");
  const [keywords, setKeywords] = useState("");

  const create = useMutation({
    mutationFn: () =>
      api.createSkill(agentId, {
        id: id.trim(),
        name: name.trim(),
        priority: Number(priority) || 0,
        keywords: keywords
          .split(",")
          .map((k) => k.trim())
          .filter(Boolean),
      }),
    onSuccess: () => {
      toast(`Created skill ${id}`, "success");
      setId("");
      setName("");
      setPriority("0");
      setKeywords("");
      onCreated();
      onClose();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <Dialog open={open} onClose={onClose} title="New skill">
      <div className="flex flex-col gap-3">
        <Field label="ID">
          <Input value={id} onChange={(e) => setId(e.target.value)} placeholder="my-skill" />
        </Field>
        <Field label="Name">
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="My Skill" />
        </Field>
        <Field label="Priority">
          <Input
            type="number"
            value={priority}
            onChange={(e) => setPriority(e.target.value)}
          />
        </Field>
        <Field label="Keywords (comma-separated)">
          <Input
            value={keywords}
            onChange={(e) => setKeywords(e.target.value)}
            placeholder="search, web"
          />
        </Field>
        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={!id.trim() || !name.trim() || create.isPending}
            onClick={() => create.mutate()}
          >
            Create
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function EditSkillDialog({
  agentId,
  skill,
  onClose,
  onSaved,
}: {
  agentId: string;
  skill: SkillDef;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { toast } = useToast();
  // Seed the frontmatter editor from the skill's structured fields.
  const frontmatter: Record<string, unknown> = {
    id: skill.id,
    name: skill.name,
    priority: skill.priority,
    keywords: skill.keywords,
    tool_overlay: skill.tool_overlay,
    memory_namespace: skill.memory_namespace,
  };

  const save = useMutation({
    mutationFn: (value: DocEditorValue) =>
      api.putSkill(agentId, skill.id, value),
    onSuccess: () => {
      toast(`Saved skill ${skill.id}`, "success");
      onSaved();
      onClose();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <Dialog open onClose={onClose} title={`Edit SKILL.md — ${skill.name}`}>
      <DocEditor
        frontmatter={frontmatter}
        body={skill.instruction_fragment}
        sourcePath={skill.source_path}
        saving={save.isPending}
        onSave={(value) => save.mutate(value)}
      />
    </Dialog>
  );
}

function HistoryDialog({
  agentId,
  sid,
  onClose,
  onReverted,
}: {
  agentId: string;
  sid: string;
  onClose: () => void;
  onReverted: () => void;
}) {
  const { toast } = useToast();
  const history = useQuery({
    queryKey: ["agent", agentId, "history", sid],
    queryFn: () => api.skillHistory(agentId, sid),
  });

  const revert = useMutation({
    mutationFn: () => api.revertSkill(agentId, sid),
    onSuccess: () => {
      toast(`Reverted ${sid}`, "success");
      onReverted();
      onClose();
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <Dialog open onClose={onClose} title={`History — ${sid}`}>
      <div className="flex flex-col gap-3">
        {history.isLoading && <SkeletonRows rows={2} />}
        {history.error && <ErrorState error={history.error} />}
        {history.data && history.data.history.length === 0 && (
          <p className="text-xs text-muted-foreground">No history entries.</p>
        )}
        {history.data && history.data.history.length > 0 && (
          <ul className="flex flex-col gap-1 text-sm">
            {history.data.history.map((h, i) => (
              <li key={i} className="rounded-md border px-3 py-1.5 font-mono text-xs">
                {h}
              </li>
            ))}
          </ul>
        )}
        <div className="flex justify-end">
          <Button
            variant="outline"
            disabled={revert.isPending}
            onClick={() => revert.mutate()}
          >
            <Undo2 className="size-3.5" /> Revert latest
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}
