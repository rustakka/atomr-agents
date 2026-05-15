import * as React from "react";
import { cn } from "@/lib/utils";

export const Table = React.forwardRef<HTMLTableElement, React.HTMLAttributes<HTMLTableElement>>(
  ({ className, ...props }, ref) => (
    <div className="relative w-full overflow-auto">
      <table ref={ref} className={cn("w-full text-sm", className)} {...props} />
    </div>
  ),
);
Table.displayName = "Table";

export const THead = (props: React.HTMLAttributes<HTMLTableSectionElement>) => (
  <thead {...props} className={cn("[&_tr]:border-b", props.className)} />
);

export const TBody = (props: React.HTMLAttributes<HTMLTableSectionElement>) => (
  <tbody {...props} className={cn("[&_tr:last-child]:border-0", props.className)} />
);

export const Tr = (props: React.HTMLAttributes<HTMLTableRowElement>) => (
  <tr
    {...props}
    className={cn(
      "border-b transition-colors hover:bg-muted/40 data-[state=selected]:bg-muted",
      props.className,
    )}
  />
);

export const Th = (props: React.ThHTMLAttributes<HTMLTableCellElement>) => (
  <th
    {...props}
    className={cn("h-9 px-3 text-left text-xs font-medium text-muted-foreground", props.className)}
  />
);

export const Td = (props: React.TdHTMLAttributes<HTMLTableCellElement>) => (
  <td {...props} className={cn("px-3 py-2 align-middle", props.className)} />
);
