import { Route, Routes } from "react-router-dom";
import { Shell } from "@/components/layout/Shell";
import ConversationsPage from "@/pages/ConversationsPage";
import ConversationDetailPage from "@/pages/ConversationDetailPage";

export function App() {
  return (
    <Shell>
      <Routes>
        <Route path="/" element={<ConversationsPage />} />
        <Route path="/c/:id" element={<ConversationDetailPage />} />
      </Routes>
    </Shell>
  );
}
