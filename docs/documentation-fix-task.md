# Task: fix documentation consistency (docs only)

**For:** the next agent picking this up. **Scope:** Markdown documentation only.
**Deliverable:** one commit (or PR) that makes the canonical docs agree with each other.
**Source review:** the documentation-consistency review delivered in chat on 2026-10-13 (findings D1–D8 below mirror it).

---

## Hard constraints

1. **Do not modify any code.** No `.rs`, `.ts`, `.tsx`, `.toml`, `.yml`, `.json`, or script changes. If a doc claim can only be resolved by reading code, stop and list it under "Unresolved — needs owner/code check" at the end of your summary instead of guessing.
2. **Do not rewrite history.** Handovers (`docs/handover-*.md`), port reviews (`docs/port-review-*.md`, `docs/review-2026-08-21-full.md`), and published release notes (`docs/release-notes/v*.md`) are snapshots. Do not edit their factual content. The only permitted touch is adding a status banner (Task T6) or fixing a broken link.
3. **Do not edit the sample corpora** (`sample/`, `sample-ai/`) — they are fixtures, not documentation.
4. **Do not fabricate release notes** for old versions (see T2).
5. Every factual claim you write must be traceable to another document in the repo that is newer or more canonical (see the canonical map below). If two canonical docs disagree and neither is clearly newer, flag it — do not pick one silently.

## Canonical map (when documents disagree, the left column wins)

| Topic | Canonical source | Everything else should link, not restate |
|---|---|---|
| Current release / version | `docs/release-notes/` (highest `v*.md`) + `rust/BACKLOG.md` open-work header | README, AGENTS, packaging, architecture must not hardcode a "current" version (T1) |
| Release procedure | `docs/releasing.md` (after T2 it is the only checklist) | `AGENTS.md` release bullet points here |
| Open engineering work | `rust/BACKLOG.md` | `docs/mcp.md`, product plan, guides must not carry their own task-status tables |
| Product scope/backlog narrative | `docs/product-plan.md` | — |
| Architecture/retrieval design | `docs/architecture.md` + `docs/guides/hierarchical-toc.md` | README summarises in ≤5 lines and links |
| Team operations | `hosting/README.md` | `docs/hosting.md` is a future plan, not operations (T5) |
| MCP usage | `docs/mcp.md` | README links; no task IDs in user-facing text (T4) |
| Backup/migration | `docs/backup.md` | — |

## Decision gate (resolve before T3)

**D-PII — item-level PII mark.** The docs currently describe three generations of PII behaviour at once (T3). Owner direction from the review discussion: the per-item mark is likely to be **removed** (source labels stay the only trust boundary), because item-level enforcement across classify/embed/answer/backfill/MCP/direct reads is too complex for current use cases. **Confirm with the owner before writing T3 copy.** If confirmed removed: document source labels only, and describe the item mark as removed/never-enforced. If the owner instead keeps it: document it honestly as "a visible mark only — it does not restrict model providers; source labels do." Do not write copy implying enforcement exists either way.

**Resolution (owner, 2026-10-13):** deprecate the whole PII surface — the item-level mark AND the `pii` source-label option are removed from the app UI and the website (Sensitive stays the only gated label in user-facing copy). The backend keeps accepting and enforcing both (`PATCH /items/:id {is_pii}`, `pii` label validation + cloud gates, backup round-trip) as hidden compat in case the feature returns — no Rust changes. Write T3 copy as removal, not advisory.

---

## Tasks (do in this order)

### T1 — Kill hardcoded "current version" claims

Canonical docs currently disagree on what shipped: `AGENTS.md:15` and `AGENTS.md:109` say 1.0.20, `README.md:69` says 1.0.20, `docs/packaging.md:4` says 1.0.20, `docs/architecture.md:417` says 1.0.13, `rust/README.md:3` says v1.0.9 — while `docs/release-notes/v1.0.26.md` exists and `rust/BACKLOG.md` already tracks a 1.0.27 gate.

- In `AGENTS.md`, `README.md`, `docs/packaging.md`, `docs/architecture.md`, `rust/README.md`: replace "shipped/current is X.Y.Z" statements with a durable reference, e.g. "shipped — see `docs/release-notes/` for the current version" or "Rust is the shipped backend (single version source: `rust/Cargo.toml`)."
- Historical version mentions inside dated narratives (e.g. "Phase 14 shipped in 1.0.12") stay — they are history, not current claims.
- Also fix stale test-count claims you encounter in canonical docs (e.g. `AGENTS.md:19` "cargo test 59"): either delete the count or say "green" without a number. Counts rot; do not substitute a new count.

### T2 — Make `docs/releasing.md` the only release checklist

- Merge the release-notes rule from `AGENTS.md:26` into `docs/releasing.md` as explicit checklist steps: draft `docs/release-notes/vX.Y.Z.md` (Highlights / Fixed / Upgrade notes, plain language, no unexplained R-/B-codes) before tagging, and attach it after CI publishes (`gh release edit vX.Y.Z --notes-file docs/release-notes/vX.Y.Z.md`).
- Replace the literal example tag `v1.0.20` (`docs/releasing.md:29`) with `vX.Y.Z` placeholders throughout.
- Add a stale-claim sweep step: grep canonical docs for the previous version and for "shipped 1.0." before tagging (the existing step 3 grep is a starting point — extend it to cover `AGENTS.md`, `README.md`, `docs/packaging.md`, `docs/architecture.md`, `rust/README.md`).
- Shrink the `AGENTS.md` release bullet to a one-line pointer to `docs/releasing.md`.
- Missing release notes for tags before v1.0.22: do **not** backfill from memory. Add one line to `docs/releasing.md` noting notes exist from v1.0.22 onward, or list the gap under Unresolved for the owner.

### T3 — One PII story (after D-PII is resolved)

Sweep these current-facing files so they tell exactly one story (per D-PII):

- `README.md:199` — the API list still documents `/pii/*` endpoints; `README.md:214` and `README.md:220` describe chunk-level auto-flagging.
- `docs/architecture.md:393` — security section describes `chunks.is_pii` auto-flagging as current behaviour.
- `docs/gated-access-spec.md:157` — describes `?gate=provider` as shipped Phase 1. This spec is largely superseded: add a top banner "Status: Part A shipped in reduced form (source-label gate on `/qa` + `/qa/search` only); Part B is backlog; the `?gate=provider` design was retired by R35.1 — see `rust/BACKLOG.md`" and leave the body as design history.
- `docs/guides/hierarchical-toc.md:43` — references the old `is_pii` gate.
- Ground truth for the sweep (from `rust/BACKLOG.md:75`, R35.1): the auto-detect stack (`/pii/*`, auto-flagging, `?gate=provider`) was deleted; answer/search gating is source-label-only; source labels + `public_only` + cloud-trust are the live model.

### T4 — De-duplicate retrieval, connectors, and MCP status

- **Retrieval:** trim the detailed ranking description in `README.md:150` to a short paragraph ending with links to `docs/architecture.md` §4 and `docs/guides/hierarchical-toc.md`. Resolve the cleaning contradiction: `README.md:128` says clean-before-chunk, `docs/architecture.md:86` calls full cleaning a known Rust gap, `rust/BACKLOG.md:70` records the cleaning port as shipped — architecture is the stale one; update it (docs-only wording: describe cleaning as shipped per the backlog log, without inventing implementation detail).
- **Connectors:** fix `README.md:122` (flow diagram lists only folder/Jira) and the API summary starting at `README.md:188` so all five connectors (folder, Drive, Jira, Linear, REST) are represented or explicitly linked. Add `docs/guides/rest-api.md` and `docs/guides/obsidian-vault.md` to the Docs list at `README.md:50`.
- **MCP:** fix "MCP sidecar zips" at `README.md:54` (single binary, `anchorcore mcp` mode). Remove volatile task-status detail from `docs/mcp.md:173` (milestones table) — replace open-task enumerations with "see `rust/BACKLOG.md` Phase 20" so it cannot drift again.

### T5 — Disambiguate hosting docs

- Rename `docs/hosting.md` to `docs/hosted-cloud-plan.md` (or, if renaming breaks links you cannot find, keep the filename) and replace its header with: "Status: **future plan** for the hosted cloud tier — not operating instructions. Team self-hosting today: one container, SQLite — see `hosting/README.md`." Update inbound links (`docs/architecture.md`, `docs/product-plan.md`, anywhere else a grep finds).
- Confirm `README.md` and `docs/architecture.md` only ever link `hosting/README.md` for Team operations.

### T6 — Documentation index + status banners

- Create `docs/README.md`: an index classifying the major documents as **Current** (README-linked guides, architecture, backup, mcp, updates, tray, releasing), **Plans** (product-plan, plan-*.md, gated-access-spec, hosted-cloud-plan, v2v3-scope), **Historical** (handover-*.md, port-review-*.md, review-2026-08-21-full.md, rust-port.md, archived-python.md, b48-clean-machine.md), and **Release notes**. Link it from the top-level `README.md` Docs list.
- Add a one-line status banner to each plan/spec that lacks one, in this form: `> Status: <current | plan | historical snapshot (YYYY-MM-DD) | superseded by <link>>`. Do not add banners to handovers one by one if the index classification covers them; do add them to `docs/gated-access-spec.md` (T3) and `docs/hosted-cloud-plan.md` (T5), which readers can mistake for current instructions.

### T7 — Final sweep and verification (all must pass)

Run from the repo root and paste the outputs into your summary:

```bash
# 1. Only Markdown changed
git status --porcelain   # expect: only *.md files (plus this task file marked done)

# 2. No stale current-version claims left in canonical docs
git grep -n "shipped 1\.0\.\|shipped as of\|shipped (1\.0" -- AGENTS.md README.md docs/packaging.md docs/architecture.md rust/README.md docs/releasing.md
# expect: no hits that claim a current version (historical "shipped in 1.0.12" is fine — eyeball each hit)

# 3. No retired-PII surface presented as current in canonical docs
git grep -n "/pii/\|auto-flag\|gate=provider" -- README.md docs/architecture.md docs/mcp.md docs/guides/hierarchical-toc.md
# expect: zero hits, or hits explicitly marked historical/retired

# 4. Relative links still resolve (python3, no deps)
python3 - << 'EOF'
import re, pathlib, subprocess, os, sys
files = subprocess.check_output(['git','ls-files','*.md'], text=True).splitlines()
bad = []
for f in files:
    for m in re.finditer(r'\[[^\]]*\]\(([^)]+)\)', pathlib.Path(f).read_text(errors='ignore')):
        link = m.group(1).strip().strip('<>')
        if not link or link.startswith(('http://','https://','mailto:','#','/Users/')): continue
        target = link.split('#')[0]
        if target and not os.path.exists(pathlib.Path(f).parent / target): bad.append((f, link))
print('\n'.join(f'{f}: {l}' for f, l in bad) or 'all relative links OK'); sys.exit(1 if bad else 0)
EOF

# 5. No code touched
git diff --stat   # Markdown only
```

## Definition of done

- [ ] T1–T6 complete; every Definition-of-done grep above passes or each remaining hit is justified in the summary.
- [ ] `git diff` touches Markdown files only.
- [ ] D-PII was confirmed with the owner, and the copy written matches the decision (removal vs advisory mark).
- [ ] Summary lists: files changed (one line each), the canonical-map choices made where docs disagreed, and an "Unresolved — needs owner/code check" section for anything you refused to guess.
- [ ] This task file gets a final line: `Status: DONE (date, commit)` — or is deleted on merge, at the owner's preference.

## Out of scope

Fixing the underlying product questions (whether the PII mark is removed in code, retrieval behaviour, release automation), editing `AGENTS.md` gotchas beyond the version/test-count claims in T1, the website (`website/`), and the agent skills under `.agents/skills/` (mention drift you notice in the summary; do not edit them here).
