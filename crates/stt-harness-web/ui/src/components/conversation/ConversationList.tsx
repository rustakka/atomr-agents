import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { formatDuration } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";

/** Table of all stored conversations; a row click opens the detail view. */
export function ConversationList() {
  const navigate = useNavigate();
  const { data = [], isLoading, error } = useQuery({
    queryKey: ["conversations"],
    queryFn: api.listConversations,
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>
          Conversations <Badge variant="outline">{data.length}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent>
        <Table>
          <THead>
            <Tr>
              <Th>ID</Th>
              <Th>Backend</Th>
              <Th>Language</Th>
              <Th>Turns</Th>
              <Th>Speakers</Th>
              <Th>Duration</Th>
            </Tr>
          </THead>
          <TBody>
            {isLoading &&
              Array.from({ length: 3 }).map((_, i) => (
                <Tr key={i}>
                  <Td colSpan={6}>
                    <Skeleton className="h-5 w-full" />
                  </Td>
                </Tr>
              ))}
            {error && (
              <Tr>
                <Td colSpan={6} className="py-6 text-center text-destructive">
                  {(error as Error).message}
                </Td>
              </Tr>
            )}
            {!isLoading && !error && data.length === 0 && (
              <Tr>
                <Td colSpan={6} className="py-6 text-center text-muted-foreground">
                  no conversations yet
                </Td>
              </Tr>
            )}
            {data.map((c) => (
              <Tr
                key={c.id}
                className="cursor-pointer"
                onClick={() => navigate(`/c/${encodeURIComponent(c.id)}`)}
              >
                <Td className="font-mono text-xs">{c.id}</Td>
                <Td>
                  {c.backend ? (
                    <Badge variant="outline">{c.backend}</Badge>
                  ) : (
                    <span className="text-muted-foreground">—</span>
                  )}
                </Td>
                <Td className="text-muted-foreground">{c.language ?? "—"}</Td>
                <Td className="tabular-nums">{c.turn_count}</Td>
                <Td className="tabular-nums">{c.speaker_count}</Td>
                <Td className="tabular-nums text-muted-foreground">
                  {formatDuration(c.total_audio_secs)}
                </Td>
              </Tr>
            ))}
          </TBody>
        </Table>
      </CardContent>
    </Card>
  );
}
