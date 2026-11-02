import type { Plugin } from "vite";
import { neuronSvg } from "../src/heroArtDraw";

// The home hero's line-art band is drawn once here, at build (and in dev), not in the
// browser: <!--hero:neurons--> becomes the static SVG markup; src/heroArt.ts only animates it.
export function heroArt(): Plugin {
  const svg = neuronSvg();
  return {
    name: "anchorcore-hero-art",
    transformIndexHtml: {
      order: "pre",
      handler(html) {
        return html.replace("<!--hero:neurons-->", svg);
      },
    },
  };
}
