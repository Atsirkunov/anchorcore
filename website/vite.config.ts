import { defineConfig } from "vite";
export default defineConfig({
  // allow ?raw imports of ../docs/*.md so repo markdown stays the source of
  // truth while the site renders it on-domain (dev only; build reads freely)
  server: { port: 5174, fs: { allow: [".."] } },
  build: { outDir: "dist" }
});
