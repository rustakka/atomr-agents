import type { Action, ActionStatus, Attendee } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";

interface ActionsTableProps {
  actions: Action[];
  attendees: Attendee[];
  onUpdate: (
    actionId: string,
    body: { status?: ActionStatus; owner_attendee_id?: string | null },
  ) => void;
}

function ownerName(attendees: Attendee[], id?: string | null): string {
  if (!id) return "—";
  return attendees.find((a) => a.id === id)?.display_name ?? "—";
}

function statusVariant(s: ActionStatus): "success" | "warning" | "outline" {
  if (s === "done") return "success";
  if (s === "cancelled") return "outline";
  return "warning";
}

export function ActionsTable({ actions, attendees, onUpdate }: ActionsTableProps) {
  if (actions.length === 0) {
    return <p className="text-xs text-muted-foreground">no actions yet</p>;
  }
  return (
    <Table>
      <THead>
        <Tr>
          <Th>Action</Th>
          <Th>Owner</Th>
          <Th>Status</Th>
          <Th>Due</Th>
        </Tr>
      </THead>
      <TBody>
        {actions.map((a) => (
          <Tr key={a.id}>
            <Td className="leading-snug">
              <div>{a.description}</div>
              {a.supporting_quote && (
                <div className="mt-0.5 text-xs italic text-muted-foreground">
                  “{a.supporting_quote}”
                </div>
              )}
            </Td>
            <Td>
              <select
                value={a.owner_attendee_id ?? ""}
                onChange={(e) =>
                  onUpdate(a.id, {
                    owner_attendee_id: e.target.value || null,
                  })
                }
                className="h-7 rounded-md border border-input bg-background px-1.5 text-xs"
              >
                <option value="">{ownerName(attendees, a.owner_attendee_id)}</option>
                {attendees.map((att) => (
                  <option key={att.id} value={att.id}>
                    {att.display_name}
                  </option>
                ))}
              </select>
            </Td>
            <Td>
              <select
                value={a.status}
                onChange={(e) =>
                  onUpdate(a.id, { status: e.target.value as ActionStatus })
                }
                className="h-7 rounded-md border border-input bg-background px-1.5 text-xs"
              >
                <option value="open">open</option>
                <option value="done">done</option>
                <option value="cancelled">cancelled</option>
              </select>
              <span className="ml-1.5">
                <Badge variant={statusVariant(a.status)}>{a.status}</Badge>
              </span>
            </Td>
            <Td className="text-xs text-muted-foreground">{a.due_iso ?? "—"}</Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
