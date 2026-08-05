import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/sources": "http://localhost:8000",
      "/entities": "http://localhost:8000",
      "/review": "http://localhost:8000",
      "/qa": "http://localhost:8000",
      "/health": "http://localhost:8000",
      "/system": "http://localhost:8000",
      "/settings": "http://localhost:8000",
    },
  },
  build: {
    outDir: "dist",
  },
});
