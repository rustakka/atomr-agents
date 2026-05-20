import { useEffect, useState, type ReactNode } from "react";
import { NavLink } from "react-router-dom";
import {
  LayoutDashboard,
  Bot,
  Clock,
  Radio,
  Boxes,
  Plug,
  Activity,
  Settings,
  BookOpen,
  Moon,
  Sun,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

interface NavEntry {
  to: string;
  label: string;
  icon: LucideIcon;
  end?: boolean;
}

interface NavGroup {
  heading: string;
  entries: NavEntry[];
}

const NAV_GROUPS: NavGroup[] = [
  {
    heading: "Runtime",
    entries: [
      { to: "/", label: "Overview", icon: LayoutDashboard, end: true },
      { to: "/agents", label: "Agents", icon: Bot },
      { to: "/events", label: "Events", icon: Activity },
    ],
  },
  {
    heading: "Orchestration",
    entries: [
      { to: "/crons", label: "Crons", icon: Clock },
      { to: "/routes", label: "Channels & Routing", icon: Radio },
      { to: "/mcp", label: "MCP", icon: Plug },
    ],
  },
  {
    heading: "Catalog",
    entries: [
      { to: "/registry", label: "Registry", icon: Boxes },
      { to: "/concepts", label: "Concepts", icon: BookOpen },
      { to: "/settings", label: "Settings", icon: Settings },
    ],
  },
];

// Mobile bottom tabs show a focused subset of the top-level destinations.
const BOTTOM_TABS: NavEntry[] = [
  { to: "/", label: "Overview", icon: LayoutDashboard, end: true },
  { to: "/agents", label: "Agents", icon: Bot },
  { to: "/events", label: "Events", icon: Activity },
  { to: "/registry", label: "Registry", icon: Boxes },
  { to: "/settings", label: "Settings", icon: Settings },
];

function navLinkClass(isActive: boolean): string {
  return cn(
    "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground transition-colors",
    "hover:bg-muted hover:text-foreground",
    isActive && "bg-muted text-foreground",
  );
}

function Wordmark() {
  return (
    <span>
      <span className="text-primary">atom</span>r Host
    </span>
  );
}

function Sidebar() {
  return (
    <aside className="hidden md:flex w-56 flex-col border-r bg-card/40 px-3 py-4">
      <div className="px-2 pb-4 text-sm font-semibold tracking-wide">
        <Wordmark />
      </div>
      <nav className="flex flex-col gap-4">
        {NAV_GROUPS.map((group) => (
          <div key={group.heading} className="flex flex-col gap-0.5">
            <div className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/70">
              {group.heading}
            </div>
            {group.entries.map((entry) => {
              const Icon = entry.icon;
              return (
                <NavLink
                  key={entry.to}
                  to={entry.to}
                  end={entry.end}
                  className={({ isActive }) => navLinkClass(isActive)}
                >
                  <Icon className="size-4" aria-hidden />
                  <span>{entry.label}</span>
                </NavLink>
              );
            })}
          </div>
        ))}
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
        <Wordmark />
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

function BottomTabs() {
  return (
    <nav className="flex md:hidden items-stretch border-t bg-card/80 backdrop-blur">
      {BOTTOM_TABS.map((entry) => {
        const Icon = entry.icon;
        return (
          <NavLink
            key={entry.to}
            to={entry.to}
            end={entry.end}
            className={({ isActive }) =>
              cn(
                "flex flex-1 flex-col items-center gap-0.5 py-2 text-[10px] text-muted-foreground transition-colors",
                isActive && "text-primary",
              )
            }
          >
            <Icon className="size-5" aria-hidden />
            <span>{entry.label}</span>
          </NavLink>
        );
      })}
    </nav>
  );
}

export function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-svh w-full">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar />
        <main className="flex-1 overflow-auto p-3 md:p-6">{children}</main>
        <BottomTabs />
      </div>
    </div>
  );
}
