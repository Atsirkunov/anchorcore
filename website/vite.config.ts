import { defineConfig } from "vite";
export default defineConfig({
  // allow ?raw imports of ../docs/*.md so repo markdown stays the source of
  // truth while the site renders it on-domain (dev only; build reads freely)
  server: { port: 5174, fs: { allow: [".."] } },
  build: {
    outDir: "dist",
    rollupOptions: {
      // two static entries: the landing page and the GDPR Art. 13 notice page
      // (R18.11 — gets its own linkable route, /privacy, via its own HTML)
      input: { main: "index.html", privacy: "privacy.html" }
    }
  }
});
