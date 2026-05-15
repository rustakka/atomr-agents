import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// During `npm run dev` the SPA runs on :5173 and proxies API + WS
// traffic to the axum server (default bind 127.0.0.1:7000). In a
// production build the same server embeds and serves `dist/`.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: {
    port: 5173,
    proxy: {
      "/api": { target: "http://127.0.0.1:7000", changeOrigin: true },
      "/ws": { target: "ws://127.0.0.1:7000", ws: true, changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
});
