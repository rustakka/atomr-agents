import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";

export default function RoutesPage() {
  const routes = useQuery({ queryKey: ["routes"], queryFn: api.getRoutes });
  const channels = useQuery({ queryKey: ["channels"], queryFn: api.listChannels });

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4">
      <h1 className="text-lg font-semibold">Channels &amp; Routing</h1>

      <Card>
        <CardHeader>
          <CardTitle>Routing</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {routes.isLoading && <SkeletonRows rows={2} />}
          {routes.error && <ErrorState error={routes.error} />}
          {routes.data && (
            <>
              <div className="flex items-center gap-2 text-sm">
                <span className="text-muted-foreground">Default agent:</span>
                {routes.data.default_agent ? (
                  <Badge>{routes.data.default_agent}</Badge>
                ) : (
                  <span className="text-muted-foreground">none</span>
                )}
              </div>

              <PinTable title="Channel pins" pins={routes.data.channel_pins} keyLabel="Channel" />
              <PinTable title="Peer pins" pins={routes.data.peer_pins} keyLabel="Peer" />
            </>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Channels</CardTitle>
        </CardHeader>
        <CardContent>
          {channels.isLoading && <SkeletonRows rows={2} />}
          {channels.error && <ErrorState error={channels.error} />}
          {channels.data && channels.data.channels.length === 0 && (
            <EmptyState title="No channels" />
          )}
          {channels.data && channels.data.channels.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {channels.data.channels.map((c) => (
                <Badge key={c} variant="outline">
                  {c}
                </Badge>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function PinTable({
  title,
  pins,
  keyLabel,
}: {
  title: string;
  pins: Record<string, string>;
  keyLabel: string;
}) {
  const entries = Object.entries(pins);
  return (
    <div>
      <p className="mb-1 text-xs font-medium text-muted-foreground">{title}</p>
      {entries.length === 0 ? (
        <p className="text-xs text-muted-foreground">None</p>
      ) : (
        <Table>
          <THead>
            <Tr>
              <Th>{keyLabel}</Th>
              <Th>Agent</Th>
            </Tr>
          </THead>
          <TBody>
            {entries.map(([k, v]) => (
              <Tr key={k}>
                <Td className="font-medium">{k}</Td>
                <Td>
                  <Badge variant="outline">{v}</Badge>
                </Td>
              </Tr>
            ))}
          </TBody>
        </Table>
      )}
    </div>
  );
}
