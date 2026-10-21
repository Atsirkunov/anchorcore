import { defineConfig } from "vite";
export default defineConfig({
  // allow ?raw imports of ../docs/*.md so repo markdown stays the source of
  // truth while the site renders it on-domain (dev only; build reads freely)
  server: { port: 5174, fs: { allow: [".."] } },
  build: {
    outDir: "dist",
    rollupOptions: {
      // static entries: the landing page, the GDPR Art. 13 notice (R18.11) and the
      // audience pages /team + /hosted (R44.16); each gets its clean route from its HTML
      input: { main: "index.html", privacy: "privacy.html", team: "team.html", hosted: "hosted.html" }
    }
  }
});
