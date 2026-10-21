import type { Plugin } from "vite";
import { renderSessionHtml, SESSION } from "../src/components/session-render";

// R44.11: fills every `<!--session:<id>-->` placeholder in index.html with the
// static final frame generated from claude-session.data.json, so editing the
// data file is enough (nothing hand-pasted to drift).
export function sessionHtml(): Plugin {
  return {
    name: "anchorcore-session-html",
    transformIndexHtml: {
      order: "pre",
      handler(html) {
        return html.replace(/<!--session:([a-z0-9-]+)-->/g, (_m, id: string) => renderSessionHtml(SESSION, id));
      },
    },
  };
}
