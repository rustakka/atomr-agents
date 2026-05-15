import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { formatRelativeMs } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";

/** Table of all stored meeting analyses; a row click opens the detail view. */
export function MeetingList() {
  const navigate = useNavigate();
  const { data = [], isLoading, error } = useQuery({
    queryKey: ["meetings"],
    queryFn: api.listMeetings,
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>
          Meetings <Badge variant="outline">{data.length}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent>
        <Table>
          <THead>
            <Tr>
              <Th>ID</Th>
              <Th>Title</Th>
              <Th>State</Th>
              <Th>Attendees</Th>
              <Th>Notes</Th>
              <Th>Actions (open)</Th>
              <Th>Updated</Th>
            </Tr>
          </THead>
          <TBody>
            {isLoading &&
              Array.from({ length: 3 }).map((_, i) => (
                <Tr key={i}>
                  <Td colSpan={7}>
                    <Skeleton className="h-5 w-full" />
                  </Td>
                </Tr>
              ))}
            {error && (
              <Tr>
                <Td colSpan={7} className="py-6 text-center text-destructive">
                  {(error as Error).message}
                </Td>
              </Tr>
            )}
            {!isLoading && !error && data.length === 0 && (
              <Tr>
                <Td colSpan={7} className="py-6 text-center text-muted-foreground">
                  no meetings yet — trigger an analysis from the detail view
                </Td>
              </Tr>
            )}
            {data.map((m) => (
              <Tr
                key={m.id}
                className="cursor-pointer"
                onClick={() => navigate(`/m/${encodeURIComponent(m.id)}`)}
              >
                <Td className="font-mono text-xs">{m.id}</Td>
                <Td>{m.title ?? <span className="text-muted-foreground">—</span>}</Td>
                <Td>
                  <Badge
                    variant={
                      m.state === "final"
                        ? "success"
                        : m.state === "streaming"
                          ? "warning"
                          : "outline"
                    }
                  >
                    {m.state}
                  </Badge>
                </Td>
                <Td className="tabular-nums">{m.attendee_count}</Td>
                <Td className="tabular-nums">{m.note_count}</Td>
                <Td className="tabular-nums">
                  {m.action_count}
                  {m.open_action_count > 0 && (
                    <span className="ml-1 text-amber-500">({m.open_action_count})</span>
                  )}
                </Td>
                <Td className="text-xs text-muted-foreground">
                  {formatRelativeMs(m.updated_at_ms)}
                </Td>
              </Tr>
            ))}
          </TBody>
        </Table>
      </CardContent>
    </Card>
  );
}
