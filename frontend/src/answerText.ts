// Answer + citation text for display. Pure + unit-tested (answerText.test.ts).
// The API keeps `[match N]` marks for agents (R36.3); people never see them.
// Answers get a small, safe markdown subset (paragraphs, lists, **bold**) and
// `[S1]` markers become citation tokens — rendered as React nodes, never HTML.

const MARK_LINE = /^\s*\[\/?match \d+\]\s*$/;
const MARK_INLINE = /\s*\[\/?match \d+\]/g;

/** Text without match marks (whole mark lines dropped, inline marks removed). */
export function stripMatchMarks(text: string): string {
  return text
    .split("\n")
    .filter((l) => !MARK_LINE.test(l))
    .join("\n")
    .replace(MARK_INLINE, "");
}

export type Inline = { t: "text"; v: string } | { t: "bold"; v: string } | { t: "cite"; n: number };
export type Block = { t: "p"; parts: Inline[] } | { t: "ul" | "ol"; items: Inline[][] };

/** `**bold**` and `[S3]` within one line of text. */
export function parseInline(line: string): Inline[] {
  const out: Inline[] = [];
  const re = /\*\*(.+?)\*\*|\[S(\d+)\]/g;
  let last = 0;
  for (const m of line.matchAll(re)) {
    const at = m.index ?? 0;
    if (at > last) out.push({ t: "text", v: line.slice(last, at) });
    if (m[1] !== undefined) out.push({ t: "bold", v: m[1] });
    else out.push({ t: "cite", n: Number(m[2]) });
    last = at + m[0].length;
  }
  if (last < line.length) out.push({ t: "text", v: line.slice(last) });
  return out;
}

const BULLET = /^\s*[-*•]\s+/;
const NUMBERED = /^\s*\d+[.)]\s+/;

/** Paragraphs and lists from a model answer (blank lines separate blocks). */
export function parseAnswer(text: string): Block[] {
  const blocks: Block[] = [];
  for (const chunk of stripMatchMarks(text).split(/\n\s*\n/)) {
    const lines = chunk.split("\n").filter((l) => l.trim() !== "");
    if (lines.length === 0) continue;
    let para: string[] = [];
    let list: { t: "ul" | "ol"; items: Inline[][] } | null = null;
    const flushPara = () => {
      if (para.length) blocks.push({ t: "p", parts: parseInline(para.join(" ")) });
      para = [];
    };
    const flushList = () => {
      if (list) blocks.push(list);
      list = null;
    };
    for (const line of lines) {
      const kind = BULLET.test(line) ? "ul" : NUMBERED.test(line) ? "ol" : null;
      if (kind) {
        flushPara();
        if (!list || list.t !== kind) {
          flushList();
          list = { t: kind, items: [] };
        }
        list.items.push(parseInline(line.replace(kind === "ul" ? BULLET : NUMBERED, "")));
      } else {
        flushList();
        para.push(line.trim());
      }
    }
    flushPara();
    flushList();
  }
  return blocks;
}

/** A readable excerpt: no marks, no markdown syntax, whitespace collapsed, capped. */
export function excerpt(text: string, max = 260): string {
  const plain = stripMatchMarks(text)
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
  return plain.length > max ? `${plain.slice(0, max - 1).trimEnd()}…` : plain;
}

/** Splits `…/file.md §2` into the location and its section number. */
export function splitSourceRef(ref: string): { location: string; section: string | null } {
  const m = ref.match(/^(.*?)\s+§(\d+)$/);
  return m ? { location: m[1], section: m[2] } : { location: ref, section: null };
}

/** A location shown relative to the folder source it lives in (longest root wins). */
export function relativeTo(location: string, roots: string[]): string {
  const root = roots
    .map((r) => r.replace(/[\\/]+$/, ""))
    .filter((r) => r && (location.startsWith(`${r}/`) || location.startsWith(`${r}\\`)))
    .sort((a, b) => b.length - a.length)[0];
  return root ? location.slice(root.length + 1) : location;
}
