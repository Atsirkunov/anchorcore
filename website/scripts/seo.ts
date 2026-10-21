import type { Plugin } from "vite";
import { readFileSync } from "node:fs";

// R44.18: JSON-LD + social tags generated at build from each page's own visible
// content, so structured data cannot drift from what people (and crawlers) read.
// Dependency-free: the HTML we parse is our own hand-written markup.

const ORIGIN = "https://anchorcore.dev";
const OG_IMAGE = `${ORIGIN}/og-card.png`;
const OG_ALT = "AnchorCore: private AI search. Ask your files, get the exact source.";
const RELEASES = "https://github.com/Atsirkunov/anchorcore/releases";
const CARGO = new URL("../../rust/Cargo.toml", import.meta.url);

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

export function decode(s: string): string {
  return s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e: string) => {
    if (e[0] === "#") {
      const n = e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return String.fromCodePoint(n);
    }
    return ENTITIES[e.toLowerCase()] ?? m;
  });
}

export const text = (html: string): string => decode(html.replace(/<[^>]*>/g, "")).replace(/\s+/g, " ").trim();

export function extractFaq(html: string): { q: string; a: string }[] {
  const section = html.match(/<section id="faq">[\s\S]*?<\/section>/)?.[0] ?? "";
  return [...section.matchAll(/<div class="qa"><h3>([\s\S]*?)<\/h3><p>([\s\S]*?)<\/p><\/div>/g)].map((m) => ({
    q: text(m[1]),
    a: text(m[2]),
  }));
}

function cargoVersion(): string | undefined {
  try {
    const toml = readFileSync(CARGO, "utf8");
    const sec = toml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? "";
    return sec.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
  } catch {
    return undefined;
  }
}

const ldScript = (obj: unknown) =>
  `<script type="application/ld+json">${JSON.stringify(obj).replace(/</g, "\\u003c")}</script>`;

const attr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
const has = (html: string, key: string) => new RegExp(`<meta[^>]+(?:property|name)="${key}"`).test(html);
const meta = (html: string, name: string) => decode(html.match(new RegExp(`<meta name="${name}" content="([^"]*)"`))?.[1] ?? "");

/** Social tags every page must carry; only missing ones are added, hand/guide-written ones win. */
function socialTags(html: string): string {
  const title = decode(html.match(/<title>([\s\S]*?)<\/title>/)?.[1] ?? "").trim();
  const desc = meta(html, "description");
  const canonical = html.match(/<link rel="canonical" href="([^"]+)"/)?.[1];
  if (!title || !desc || !canonical) throw new Error(`anchorcore-seo: page lacks <title>, description or canonical`);
  const want: [string, string, string][] = [
    ["property", "og:type", "website"],
    ["property", "og:site_name", "AnchorCore"],
    ["property", "og:title", title],
    ["property", "og:description", desc],
    ["property", "og:url", canonical],
    ["property", "og:image", OG_IMAGE],
    ["property", "og:image:width", "1200"],
    ["property", "og:image:height", "630"],
    ["property", "og:image:alt", OG_ALT],
    ["name", "twitter:card", "summary_large_image"],
    ["name", "twitter:title", title],
    ["name", "twitter:description", desc],
    ["name", "twitter:image", OG_IMAGE],
    ["name", "twitter:image:alt", OG_ALT],
  ];
  return want
    .filter(([, k]) => !has(html, k))
    .map(([t, k, v]) => `<meta ${t}="${k}" content="${attr(v)}" />`)
    .join("\n  ");
}

function landingJsonLd(html: string, warn: (m: string) => void): string[] {
  const faq = extractFaq(html);
  const visible = (html.match(/<div class="qa">/g) ?? []).length;
  if (faq.length === 0 || faq.length !== visible || faq.some((e) => !e.q || !e.a)) {
    throw new Error(`anchorcore-seo: FAQ extraction failed (${faq.length} parsed, ${visible} visible, empty entries not allowed)`);
  }
  const version = cargoVersion();
  if (!version) warn("rust/Cargo.toml workspace.package.version unreadable: softwareVersion omitted from JSON-LD");
  const app = {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "AnchorCore",
    applicationCategory: "BusinessApplication",
    operatingSystem: "macOS, Windows, Linux",
    description: meta(html, "description"),
    ...(version ? { softwareVersion: version } : {}),
    downloadUrl: RELEASES,
    url: `${ORIGIN}/`,
    screenshot: OG_IMAGE,
    publisher: {
      "@type": "Person",
      name: "Aleksandr Tsirkunov",
      sameAs: "https://ee.linkedin.com/in/a-tsirkunov",
    },
    // free personal app only: Team is quote-based and has no machine-readable offer
    offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
  };
  const faqLd = {
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: faq.map((e) => ({
      "@type": "Question",
      name: e.q,
      acceptedAnswer: { "@type": "Answer", text: e.a },
    })),
  };
  return [ldScript(app), ldScript(faqLd)];
}

/** Re-read the emitted FAQPage and assert it equals the visible markup. */
function verifyFaq(html: string): void {
  const blocks = [...html.matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g)].map((m) => JSON.parse(m[1]));
  const ld = blocks.find((b) => b["@type"] === "FAQPage");
  const visible = extractFaq(html);
  const got = (ld?.mainEntity ?? []).map((e: { name: string; acceptedAnswer: { text: string } }) => ({ q: e.name, a: e.acceptedAnswer.text }));
  if (!ld || got.length === 0 || JSON.stringify(got) !== JSON.stringify(visible)) {
    throw new Error("anchorcore-seo: FAQPage JSON-LD does not match the visible FAQ");
  }
}

export function seo(): Plugin {
  return {
    name: "anchorcore-seo",
    transformIndexHtml: {
      order: "post",
      handler(html, ctx) {
        const isLanding = ctx.path === "/index.html";
        const tags = [socialTags(html)];
        if (isLanding) tags.push(...landingJsonLd(html, (m) => console.warn(`\n[anchorcore-seo] WARNING: ${m}`)));
        const out = html.replace("</head>", `  ${tags.filter(Boolean).join("\n  ")}\n</head>`);
        if (isLanding) verifyFaq(out);
        return out;
      },
    },
  };
}
