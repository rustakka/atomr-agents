import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Table, TBody, Td, Th, THead, Tr } from "@/components/ui/table";
import { EmptyState, ErrorState, SkeletonRows } from "@/components/ui/states";

export default function ConceptsPage() {
  const concepts = useQuery({ queryKey: ["concepts"], queryFn: api.listConcepts });

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-4">
      <div>
        <h1 className="text-lg font-semibold">Concepts</h1>
        <p className="text-xs text-muted-foreground">
          The unified concept system — host primitives and where they were borrowed from.
        </p>
      </div>

      {concepts.isLoading && <SkeletonRows rows={5} />}
      {concepts.error && <ErrorState error={concepts.error} />}
      {concepts.data && concepts.data.concepts.length === 0 && (
        <EmptyState title="No concepts registered" />
      )}
      {concepts.data && concepts.data.concepts.length > 0 && (
        <Card>
          <CardContent className="pt-4">
            <Table>
              <THead>
                <Tr>
                  <Th>Label</Th>
                  <Th>Primitive</Th>
                  <Th>Borrowed from</Th>
                  <Th>API</Th>
                  <Th>UI section</Th>
                  <Th>Description</Th>
                </Tr>
              </THead>
              <TBody>
                {concepts.data.concepts.map((c) => (
                  <Tr key={c.key}>
                    <Td className="font-medium">{c.label}</Td>
                    <Td>
                      <Badge variant="outline">{c.primitive}</Badge>
                    </Td>
                    <Td className="text-muted-foreground">{c.borrowed_from}</Td>
                    <Td className="font-mono text-xs text-muted-foreground">{c.api}</Td>
                    <Td>
                      <Badge>{c.ui_section}</Badge>
                    </Td>
                    <Td className="max-w-sm text-xs text-muted-foreground">
                      {c.description}
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
