import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Save } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ErrorState, SkeletonRows } from "@/components/ui/states";
import { useToast } from "@/components/ui/toast";

export default function SettingsPage() {
  const qc = useQueryClient();
  const { toast } = useToast();
  const [yaml, setYaml] = useState("");

  const config = useQuery({ queryKey: ["config"], queryFn: api.getConfig });

  useEffect(() => {
    if (config.data) setYaml(config.data.yaml);
  }, [config.data]);

  const save = useMutation({
    mutationFn: () => api.putConfig(yaml),
    onSuccess: () => {
      toast("Config saved", "success");
      qc.invalidateQueries({ queryKey: ["config"] });
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4">
      <h1 className="text-lg font-semibold">Settings</h1>

      {config.isLoading && <SkeletonRows rows={4} />}
      {config.error && <ErrorState error={config.error} />}
      {config.data && (
        <Card>
          <CardHeader className="flex-row items-center justify-between space-y-0">
            <CardTitle>Host config (YAML)</CardTitle>
            <Button disabled={save.isPending} onClick={() => save.mutate()}>
              <Save className="size-3.5" /> {save.isPending ? "Saving…" : "Save"}
            </Button>
          </CardHeader>
          <CardContent>
            <Textarea
              value={yaml}
              spellCheck={false}
              className="min-h-[28rem]"
              onChange={(e) => setYaml(e.target.value)}
            />
          </CardContent>
        </Card>
      )}
    </div>
  );
}
