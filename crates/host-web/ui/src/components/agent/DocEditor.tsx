import { useEffect, useState } from "react";
import { ChevronDown, ChevronRight, Save } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { prettyJson } from "@/lib/utils";

export interface DocEditorValue {
  frontmatter: Record<string, unknown>;
  body: string;
}

interface Props {
  frontmatter: Record<string, unknown>;
  body: string;
  sourcePath?: string | null;
  saving?: boolean;
  onSave: (value: DocEditorValue) => void;
}

/** Editor for a Markdown doc: a collapsible JSON frontmatter textarea plus
 *  the body textarea. Used for SOUL/Rules/Memory/User docs and SKILL.md. */
export function DocEditor({ frontmatter, body, sourcePath, saving, onSave }: Props) {
  const [fmText, setFmText] = useState(() => prettyJson(frontmatter));
  const [bodyText, setBodyText] = useState(body);
  const [showFm, setShowFm] = useState(false);
  const [fmError, setFmError] = useState<string | null>(null);

  // Re-seed editor state when the upstream doc changes (e.g. after reload).
  useEffect(() => {
    setFmText(prettyJson(frontmatter));
  }, [frontmatter]);
  useEffect(() => {
    setBodyText(body);
  }, [body]);

  const save = () => {
    let parsed: Record<string, unknown> = {};
    const trimmed = fmText.trim();
    if (trimmed) {
      try {
        const value = JSON.parse(trimmed);
        if (value && typeof value === "object" && !Array.isArray(value)) {
          parsed = value as Record<string, unknown>;
        } else {
          setFmError("Frontmatter must be a JSON object.");
          return;
        }
      } catch {
        setFmError("Frontmatter is not valid JSON.");
        return;
      }
    }
    setFmError(null);
    onSave({ frontmatter: parsed, body: bodyText });
  };

  return (
    <div className="flex flex-col gap-3">
      <div>
        <button
          type="button"
          className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
          onClick={() => setShowFm((s) => !s)}
        >
          {showFm ? (
            <ChevronDown className="size-3.5" />
          ) : (
            <ChevronRight className="size-3.5" />
          )}
          Frontmatter (JSON)
        </button>
        {showFm && (
          <div className="mt-2">
            <Textarea
              value={fmText}
              onChange={(e) => setFmText(e.target.value)}
              spellCheck={false}
              className="min-h-[8rem]"
            />
            {fmError && (
              <p className="mt-1 text-xs text-destructive">{fmError}</p>
            )}
          </div>
        )}
      </div>

      <div>
        <label className="mb-1 block text-xs font-medium text-muted-foreground">
          Body
        </label>
        <Textarea
          value={bodyText}
          onChange={(e) => setBodyText(e.target.value)}
          spellCheck={false}
          className="min-h-[20rem]"
        />
      </div>

      <div className="flex items-center justify-between">
        <span className="truncate text-xs text-muted-foreground/70">
          {sourcePath ?? ""}
        </span>
        <Button onClick={save} disabled={saving}>
          <Save className="size-3.5" /> {saving ? "Saving…" : "Save"}
        </Button>
      </div>
    </div>
  );
}
