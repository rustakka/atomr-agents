import { Route, Routes } from "react-router-dom";
import { Shell } from "@/components/layout/Shell";
import OverviewPage from "@/pages/OverviewPage";
import AgentsPage from "@/pages/AgentsPage";
import AgentDetailPage from "@/pages/AgentDetailPage";
import CronsPage from "@/pages/CronsPage";
import RoutesPage from "@/pages/RoutesPage";
import RegistryPage from "@/pages/RegistryPage";
import McpPage from "@/pages/McpPage";
import EventsPage from "@/pages/EventsPage";
import SettingsPage from "@/pages/SettingsPage";
import ConceptsPage from "@/pages/ConceptsPage";

export function App() {
  return (
    <Shell>
      <Routes>
        <Route path="/" element={<OverviewPage />} />
        <Route path="/agents" element={<AgentsPage />} />
        <Route path="/agents/:id" element={<AgentDetailPage />} />
        <Route path="/crons" element={<CronsPage />} />
        <Route path="/routes" element={<RoutesPage />} />
        <Route path="/registry" element={<RegistryPage />} />
        <Route path="/mcp" element={<McpPage />} />
        <Route path="/events" element={<EventsPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="/concepts" element={<ConceptsPage />} />
      </Routes>
    </Shell>
  );
}
