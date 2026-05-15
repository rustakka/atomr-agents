import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
// `npm run dev` runs the SPA on :5174 and proxies API + WS traffic to
// the axum server (default bind 127.0.0.1:7100). The production build
// serves `dist/` from the same axum process via rust-embed.
export default defineConfig({
    plugins: [react()],
    resolve: {
        alias: { "@": path.resolve(__dirname, "./src") },
    },
    server: {
        port: 5174,
        proxy: {
            "/api": { target: "http://127.0.0.1:7100", changeOrigin: true },
            "/ws": { target: "ws://127.0.0.1:7100", ws: true, changeOrigin: true },
        },
    },
    build: {
        outDir: "dist",
        sourcemap: true,
    },
});
