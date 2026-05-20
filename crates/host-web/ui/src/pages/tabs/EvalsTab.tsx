import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Play, ChevronRight } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";
import { JsonView } from "@/components/ui/json-view";
import { useToast } from "@/components/ui/toast";
import type { EvalRun } from "@/lib/apiTypes";

export function EvalsTab({ agentId }: { agentId: string }) {
  const { toast } = useToast();
  const [openSuite, setOpenSuite] = useState<string | null>(null);
  const [run, setRun] = useState<EvalRun | null>(null);

  const suites = useQuery({ queryKey: ["evals"], queryFn: api.listEvals });

  const suite = useQuery({
    queryKey: ["eval", openSuite],
    queryFn: () => api.getEvalSuite(openSuite as string),
    enabled: !!openSuite,
  });

  const runEval = useMutation({
    mutationFn: (suiteId: string) => api.runEval(suiteId, agentId),
    onSuccess: (res) => {
      setRun(res);
      toast(`Ran ${res.suite_id}: ${res.passed}/${res.total} passed`, "success");
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  if (suites.isLoading) return <SkeletonRows rows={3} />;
  if (suites.error) return <ErrorState error={suites.error} />;
  if (!suites.data || suites.data.suites.length === 0)
    return <EmptyState title="No eval suites" hint="Add suites under the host evals directory." />;

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Suites</CardTitle>
        </CardHeader>
        <CardContent>
          <ul className="flex flex-col gap-1.5">
            {suites.data.suites.map((s) => (
              <li
                key={s}
                className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
              >
                <button
                  type="button"
                  className="flex items-center gap-1 font-medium hover:text-primary"
                  onClick={() => {
                    setOpenSuite((cur) => (cur === s ? null : s));
                    setRun(null);
                  }}
                >
                  <ChevronRight className="size-3.5" /> {s}
                </button>
                <Button
                  size="sm"
                  className="ml-auto"
                  disabled={runEval.isPending}
                  onClick={() => {
                    setOpenSuite(s);
                    runEval.mutate(s);
                  }}
                >
                  <Play className="size-3.5" /> Run
                </Button>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>

      {openSuite && (
        <Card>
          <CardHeader>
            <CardTitle>Suite: {openSuite}</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            {suite.isLoading && <SkeletonRows rows={2} />}
            {suite.error && <ErrorState error={suite.error} />}
            {suite.data && (
              <>
                <p className="text-xs text-muted-foreground">
                  Scorer: <Badge variant="outline">{suite.data.scorer}</Badge>
                  {suite.data.description ? ` — ${suite.data.description}` : ""}
                </p>
                <Table>
                  <THead>
                    <Tr>
                      <Th>Case</Th>
                      <Th>Input</Th>
                      <Th>Expected</Th>
                    </Tr>
                  </THead>
                  <TBody>
                    {suite.data.cases.map((c) => (
                      <Tr key={c.id}>
                        <Td className="font-medium">{c.id}</Td>
                        <Td>
                          <JsonView value={c.input} className="max-h-24 max-w-xs" />
                        </Td>
                        <Td>
                          <JsonView value={c.expected} className="max-h-24 max-w-xs" />
                        </Td>
                      </Tr>
                    ))}
                  </TBody>
                </Table>
              </>
            )}
          </CardContent>
        </Card>
      )}

      {run && (
        <Card>
          <CardHeader className="flex-row items-center justify-between space-y-0">
            <CardTitle>Run results</CardTitle>
            <Badge
              variant={run.passed === run.total ? "success" : "warning"}
            >
              {run.passed}/{run.total} passed
              {run.total > 0
                ? ` (${Math.round((run.passed / run.total) * 100)}%)`
                : ""}
            </Badge>
          </CardHeader>
          <CardContent>
            <Table>
              <THead>
                <Tr>
                  <Th>Case</Th>
                  <Th>Result</Th>
                  <Th className="text-right">Score</Th>
                  <Th>Reason</Th>
                  <Th>Output</Th>
                </Tr>
              </THead>
              <TBody>
                {run.results.map((r) => (
                  <Tr key={r.case_id}>
                    <Td className="font-medium">{r.case_id}</Td>
                    <Td>
                      <Badge variant={r.passed ? "success" : "destructive"}>
                        {r.passed ? "pass" : "fail"}
                      </Badge>
                    </Td>
                    <Td className="text-right">{r.score.toFixed(2)}</Td>
                    <Td className="text-muted-foreground">{r.reason ?? "—"}</Td>
                    <Td>
                      <JsonView value={r.output} className="max-h-24 max-w-xs" />
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
