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
/** `start` is the first number of an ordered list (model output may begin at 3). */
type ListBlock = { t: "ul"; items: Inline[][] } | { t: "ol"; items: Inline[][]; start: number };
export type Block = { t: "p"; parts: Inline[] } | ListBlock;

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
const NUMBERED = /^\s*(\d+)[.)]\s+/;
const HEADING = /^\s*#{1,6}\s+(.*?)\s*#*\s*$/;

type Line = { kind: "ul" | "ol" | "h" | "text"; text: string; n: number };

function classify(raw: string): Line {
  const num = raw.match(NUMBERED);
  if (num) return { kind: "ol", text: raw.replace(NUMBERED, ""), n: Number(num[1]) };
  if (BULLET.test(raw)) return { kind: "ul", text: raw.replace(BULLET, ""), n: 0 };
  const h = raw.match(HEADING);
  if (h) return { kind: "h", text: h[1], n: 0 };
  return { kind: "text", text: raw.trim(), n: 0 };
}

/** Builds blocks from lines. Rules (Wave 1, UX audit #1):
 *  - a heading is a bold run-in: it leads the paragraph that follows it,
 *    never a `#` and never its own block;
 *  - hard-wrapped lines re-flow: a plain line joins the paragraph or the
 *    list item above it (no blank line in between);
 *  - an ordered list keeps going across blank lines and keeps the first
 *    number it was given, so "1. … (blank) 1. … (blank) 1. …" reads 1, 2, 3. */
class Builder {
  blocks: Block[] = [];
  private para: string[] = [];
  private lead: string | null = null;
  private list: ListBlock | null = null;

  add(line: Line, afterBlank: boolean) {
    if (line.kind === "h") return this.heading(line.text);
    if (line.kind === "text") {
      if (this.list && !afterBlank) return this.continueItem(line.text);
      this.closeList();
      this.para.push(line.text);
      return;
    }
    this.flushPara();
    this.listItem(line);
  }

  finish() {
    this.flushPara();
    this.closeList();
    if (this.lead) this.blocks.push({ t: "p", parts: [{ t: "bold", v: this.lead }] });
    this.lead = null;
  }

  /** Blank line: ends a paragraph; a list stays open for the next item. */
  blank() {
    this.flushPara();
  }

  private heading(text: string) {
    this.flushPara();
    this.closeList();
    if (this.lead) this.blocks.push({ t: "p", parts: [{ t: "bold", v: this.lead }] });
    this.lead = text.replace(/\*\*/g, "") || null;
  }

  private continueItem(text: string) {
    const items = this.list?.items;
    if (!items || items.length === 0) return;
    const last = items[items.length - 1];
    items[items.length - 1] = [...last, { t: "text", v: " " }, ...parseInline(text)];
  }

  private listItem(line: Line) {
    const kind = line.kind as "ul" | "ol";
    if (!this.list || this.list.t !== kind) {
      this.closeList();
      this.flushLead();
      this.list = kind === "ol" ? { t: "ol", items: [], start: line.n } : { t: "ul", items: [] };
    }
    this.list.items.push(parseInline(line.text));
  }

  private flushLead() {
    if (this.lead) this.blocks.push({ t: "p", parts: [{ t: "bold", v: this.lead }] });
    this.lead = null;
  }

  private flushPara() {
    if (this.para.length === 0) return;
    const parts = parseInline(this.para.join(" "));
    this.blocks.push({ t: "p", parts: this.lead ? [{ t: "bold", v: this.lead }, { t: "text", v: " " }, ...parts] : parts });
    this.lead = null;
    this.para = [];
  }

  private closeList() {
    if (this.list) this.blocks.push(this.list);
    this.list = null;
  }
}

/** Paragraphs and lists from a model answer (blank lines separate blocks). */
export function parseAnswer(text: string): Block[] {
  const b = new Builder();
  let afterBlank = false;
  for (const raw of stripMatchMarks(text).split("\n")) {
    if (raw.trim() === "") {
      b.blank();
      afterBlank = true;
      continue;
    }
    b.add(classify(raw), afterBlank);
    afterBlank = false;
  }
  b.finish();
  return b.blocks;
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
