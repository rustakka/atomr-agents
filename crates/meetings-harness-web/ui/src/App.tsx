import { Route, Routes } from "react-router-dom";
import { Shell } from "@/components/layout/Shell";
import MeetingsPage from "@/pages/MeetingsPage";
import MeetingDetailPage from "@/pages/MeetingDetailPage";

export function App() {
  return (
    <Shell>
      <Routes>
        <Route path="/" element={<MeetingsPage />} />
        <Route path="/m/:id" element={<MeetingDetailPage />} />
      </Routes>
    </Shell>
  );
}
