# Handover 2026-11-02: UX audit, redesign proposal and mockups

For the next agent. This was a research-and-design session: **no product code was changed and nothing was committed.** Read `AGENTS.md` first, then this file, then the two docs below.

## 1. What exists

| Artifact | Where | Status |
|---|---|---|
| UX audit (findings, severity, evidence) | `docs/ux-audit-2026-11.md` | untracked, not committed |
| Redesign proposal (principles, palette history, IA, wireframes, rollout, owner calls) | `docs/ux-proposal-2026-11.md` | untracked, not committed |
| Visual mockups (7 boards, Archive tokens) | https://claude.ai/artifact/DBd4MGhw9nZhGz5VULYLbS (private to the owner) | published, unrendered/unverified by me |
| This handover | `docs/handover-2026-11-02-ux-redesign.md` | untracked |

Mockup boards: Ask (answer + collapsed history) · Ask without an answer model (evidence view) · Library (list first + Add drawer) · Agents (new tab) · Settings (rail, autosave) · Dark palette today vs retuned · Website home (6 sections). Numbers in them ("1,240 issues", "214 documents", "Qwen 3 8B", "12 searches today") are filler; `[SIZE]`, `[PHOTO]`, `[FOUNDER NOTE]` are deliberate placeholders.

## 2. Tree state when I stopped (shared checkout — see memory `shared-checkout`)

- Branch **`rust/R48.1`** at `cee03e7`, owned by **another session** working Phase 48 (whole-app review). It has uncommitted edits in `frontend/src/{api.ts,types.ts,tabs/AskTab.tsx,tabs/SourcesTab.tsx}`, `rust/BACKLOG.md`, `sources.rs`, setup skill. **Do not reset, checkout or stage those.**
- Branch `fluid-functionalism` holds an unrelated UI experiment (`frontend/src/fluid*`, `fluid-*.html`); the audit excluded it.
- My three docs are the only files I created. To commit them, use a worktree off `origin/main` (`git worktree add ../anchorcore-ux origin/main -b ux/audit-proposal`), copy the three files in, commit there. Docs-only, so CI skips it.
- Phase 48 overlaps the UI work: **R48.7** (frontend transport, Ask 30 s timeout vs server 120 s) and **R48.8** (one `errMsg()`) touch the same files as Wave 1. Land after them or coordinate.

## 3. Key findings (full list in the audit)

1. **Ask without an answer model** shows 4 warnings and raw markdown. Root cause: `answer.rs` fallback returns `"Answer for: {q}\n\n[No model key configured. Matching context:]\n\n{context}"` cut at **1,500 chars** (why text ends mid-word), and `AnswerView.tsx` renders it as an answer while `modeBannerNotes` + the App banner add three more warnings.
2. **Tab switch loses the Ask conversation**: `App.tsx` renders tabs conditionally (`{tab === "ask" && <AskTab/>}`), `AskTab` keeps `turns` in local `useState`. Verified live.
3. **No URL routing**: `const [tab, setTab] = useState<Tab>("ask")` in `App.tsx`. No reload/back/deep link.
4. **Starter chips** are `SAMPLE_QUESTIONS` in `AskTab.tsx`, shown for every corpus; click only fills the input.
5. **Stale claims**: site still lists the `pii` label (10× in HTML + `public/llms.txt`; merged into `sensitive` by R35.4 / migration 29); subpage footers say "v1 local"; `McpConnect.tsx` says "Tools: ask + search" (server has 12); `BackupCard.tsx` mentions "review state" (queue removed; `/review/*` API removed in R48.5); `SettingsTab.tsx` TOC & Tags text describes keyword "proposals" + a cosine threshold — check whether `ANCHOR_TAG_REUSE_THRESHOLD` still does anything after R45.18.
6. System tab prints `redacted` (the `data_dir` from `/health`) as if it were data; "Answer model: missing · gpt-4o-mini" contradicts Settings showing Local Ollama.
7. Website: `§1–§8` heading counters (`style.css` `main{counter-reset:sec}`), mono eyebrow pills, 70 em-dashes in HTML (175 in app strings), hero button scrolls instead of downloading (`main.ts` `initDownloadLinks` only rewrites `cta-roadmap`), button never names the OS.

## 4. Design history you must respect (sources checked)

- Palette lineage: indigo → **Stone & Sage** (R12.1, handover `docs/archive/handover-2026-08-28-v1.0.11.md`: "indigo reads as chatbot/SaaS") → **Archive** (R45.15 + `871c8df`, 2026-10-22: ink on cool paper, oxblood `#8C2F2B` = "this is sourced", IBM Plex self-hosted for the no-third-parties promise, 2 px radius, no emoji/stripes). Rules live in `docs/design-system.md`. There is **no `decisions.md` entry** for Archive — rationale is only in commits/handovers.
- The proposal keeps Archive; diagnosis is execution: the **dark set** (`#151412` + salmon `#E39A8F`) is what reads as AI-generated, oxblood is overused, the app ignores R44.14's "mono only for code", and only Ask got a structural redesign.
- Deliberate choices kept: newest-first Ask (design-system §4) — collapse older pairs instead of reversing; serif answer text; R44.9 hero line; quote-only Team pricing (R44.16).
- `AGENTS.md` owner decisions (model-free ingest, people-owned vocabulary/captions, one Sensitive concept, two answer modes, manual-first updates) are untouched by every proposal.

## 5. Owner calls pending (need a `rust/docs/decisions.md` entry each before shipping)

1. Links in ink; oxblood for citations only (changes design-system rule 2).
2. Website defaults to Light (app stays Auto; theme switch moves to Settings → Appearance / site footer).
3. Drop `§` counters on the marketing site (partly reverses `871c8df`).
4. App IA: Ask · Library · Records · Agents · Settings; System folds into Settings; Glossary moves to Settings.
5. Dark retune: neutral paper `#17171A`, surface `#1F1F23`, cite `#CC8A80` (≈5.6:1 on surface, estimated — run a contrast check).

None asked yet. Wave 1 needs none of them.

## 6. Suggested next step: Wave 1 (~2 d, no owner call)

File it as a new backlog phase (next free number after 48 — check `rust/BACKLOG.md` first; another session is editing it). Proposed rows:

| Row | Work | Files |
|---|---|---|
| a | Hash routes (`#/ask`, `#/library`…, `#/settings/<section>`) + keep tab state across switches (lift Ask turns into a context or keep tabs mounted and hidden) | `App.tsx`, `AskTab.tsx`, new `useRoute` hook |
| b | No-model local answers render the agent-mode evidence view ("Matching passages"); stop showing the fallback string. Prefer a machine-readable mode from the server over string matching; if `answer.rs` changes, keep `/qa` contracts and the MCP `ask` text sensible, extend `tests/api_*.rs` | `AnswerView.tsx`, `answerMode.ts`, `modelHealth.ts`, `answer.rs` |
| c | One notice rule: App-level red banner → header status dot; Ask-only dismissible line; agent mode shows nothing | `App.tsx`, `modelHealth.ts` |
| d | Starters only when the sample corpus is the only source; build from the person's records otherwise; click runs the question | `AskTab.tsx` (+ `/decisions` / `/actions` reads) |
| e | Copy fixes: remove `pii` from site + `llms.txt`; drop "v1 local" footer; MCP tool list; backup "review state"; hide `redacted`; decide the tag-threshold setting | `website/*.html`, `website/public/llms.txt`, `McpConnect.tsx`, `BackupCard.tsx`, `SystemTab.tsx`, `SettingsTab.tsx` |
| f | `answerText.ts`: headings as bold run-ins, real ordered lists, reflow hard-wrapped lines | `answerText.ts` + test |

Gates (from `AGENTS.md`): `cd frontend && npm run build` + vitest + lint; `cargo test -p anchorcore` with `ANCHOR_SECRETS_NO_KEYRING=1`; `RUSTFLAGS="-D warnings" cargo check`; lizard clean; golden eval if `answer.rs`/`retrieval.rs` change; website `npm run build` (runs `check-session-quotes` + `check-links`). Colours only via `theme.ts` tokens. Update `docs/design-system.md` in the same commit as any rule change.

## 7. How to reproduce what I saw

- App: a v1.6.6 instance was on `127.0.0.1:8000` (another session's test instance, data dir under a scratchpad; sample corpus synced, no answer model). Your own: `cargo run -p anchorcore -- --port 8000 --data-dir <tmp>`, add `sample/`, leave the answer model unset, ask "Who owns billing migration?".
- Site: https://anchorcore.dev with an OS in dark mode (Auto) to see the palette problem; source in `website/`.
- Not checked: phone widths (the browser would not resize), Windows/Linux builds, the onboarding wizard live (code read only: `wizard/wizardText.ts`).
