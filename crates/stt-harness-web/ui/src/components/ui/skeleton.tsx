import * as React from "react";
import { cn } from "@/lib/utils";

// Vendored from atomr-dashboard's design system.
export function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("animate-pulse rounded-md bg-muted", className)} {...props} />
  );
}
