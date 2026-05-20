import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// `npm run dev` runs the SPA on :5175 and proxies API + SSE traffic to
// the host-web axum server (default bind 127.0.0.1:7400). The production
// build serves `dist/` from the same axum process via rust-embed.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: {
    port: 5175,
    proxy: {
      "/api": { target: "http://127.0.0.1:7400", changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
});
