import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Send, Info } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useToast } from "@/components/ui/toast";

interface ChatMessage {
  role: "user" | "agent";
  text: string;
}

export function ChatTab({ agentId }: { agentId: string }) {
  const { toast } = useToast();
  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  const send = useMutation({
    mutationFn: (message: string) => api.chat(agentId, message),
    onSuccess: (res) => {
      setMessages((prev) => [...prev, { role: "agent", text: res.reply }]);
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });

  const submit = () => {
    const text = input.trim();
    if (!text || send.isPending) return;
    setMessages((prev) => [...prev, { role: "user", text }]);
    setInput("");
    send.mutate(text);
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-500">
        <Info className="mt-0.5 size-4 shrink-0" />
        <span>Preview only — deterministic reply, no live LLM turn yet.</span>
      </div>

      <div className="flex min-h-[16rem] flex-col gap-2 rounded-md border bg-card/40 p-3">
        {messages.length === 0 ? (
          <p className="m-auto text-xs text-muted-foreground">
            Send a message to preview the agent's reply.
          </p>
        ) : (
          messages.map((m, i) => (
            <div
              key={i}
              className={
                m.role === "user"
                  ? "self-end max-w-[80%] rounded-lg bg-primary px-3 py-2 text-sm text-primary-foreground"
                  : "self-start max-w-[80%] rounded-lg border bg-card px-3 py-2 text-sm"
              }
            >
              {m.text}
            </div>
          ))
        )}
        {send.isPending && (
          <div className="self-start max-w-[80%] rounded-lg border bg-card px-3 py-2 text-sm text-muted-foreground">
            …
          </div>
        )}
      </div>

      <div className="flex items-end gap-2">
        <Textarea
          value={input}
          placeholder="Message the agent…"
          className="min-h-[2.5rem] font-sans"
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
        />
        <Button onClick={submit} disabled={send.isPending || !input.trim()}>
          <Send className="size-3.5" /> Send
        </Button>
      </div>
    </div>
  );
}
