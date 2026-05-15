import { useEffect, useState, type ReactNode } from "react";
import { NavLink } from "react-router-dom";
import { ListChecks, Moon, Sun } from "lucide-react";
import { cn } from "@/lib/utils";

function Sidebar() {
  return (
    <aside className="hidden md:flex w-56 flex-col border-r bg-card/40 px-3 py-4">
      <div className="px-2 pb-4 text-sm font-semibold tracking-wide">
        <span className="text-primary">Meetings</span> Harness
      </div>
      <nav className="flex flex-col gap-0.5">
        <NavLink
          to="/"
          className={({ isActive }) =>
            cn(
              "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors",
              "hover:bg-muted hover:text-foreground",
              isActive && "bg-muted text-foreground",
            )
          }
        >
          <ListChecks className="size-4" aria-hidden />
          <span>Meetings</span>
        </NavLink>
      </nav>
    </aside>
  );
}

function TopBar() {
  const [dark, setDark] = useState(() =>
    document.documentElement.classList.contains("dark"),
  );
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);

  return (
    <header className="sticky top-0 z-30 flex h-12 items-center gap-3 border-b bg-background/85 px-3 backdrop-blur">
      <div className="flex md:hidden items-center gap-1 text-sm font-semibold">
        <span className="text-primary">Meetings</span> Harness
      </div>
      <div className="ml-auto">
        <button
          type="button"
          aria-label="toggle theme"
          className="rounded-md border p-1 text-muted-foreground hover:text-foreground"
          onClick={() => setDark((d) => !d)}
        >
          {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </button>
      </div>
    </header>
  );
}

export function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-svh w-full">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar />
        <main className="flex-1 overflow-auto p-3 md:p-6">{children}</main>
      </div>
    </div>
  );
}
