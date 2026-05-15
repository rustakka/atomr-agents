import * as React from "react";
import { cn } from "@/lib/utils";

// Vendored from atomr-dashboard's design system.
type Variant = "default" | "outline" | "success" | "warning" | "destructive";

const variants: Record<Variant, string> = {
  default: "bg-primary/15 text-primary border-primary/30",
  outline: "border-border text-foreground",
  success: "bg-emerald-500/15 text-emerald-500 border-emerald-500/30",
  warning: "bg-amber-500/15 text-amber-500 border-amber-500/30",
  destructive: "bg-destructive/15 text-destructive border-destructive/30",
};

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: Variant;
}

export function Badge({ className, variant = "default", ...props }: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium",
        variants[variant],
        className,
      )}
      {...props}
    />
  );
}
