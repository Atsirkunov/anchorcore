# AnchorCore — docs index

> Status: current — this index classifies every doc as Current, Plans, Historical, or Release notes. For engineering status, `rust/BACKLOG.md` wins; for product scope, `product-plan.md` wins.

## Current (maintained references)

- [architecture.md](./architecture.md) — system view, retrieval design (§4), data model
- [mcp.md](./mcp.md) — agent connectivity (MCP): tools, transports, setup
- [backup.md](./backup.md) — backup, export & device migration (user guide + format spec)
- [releasing.md](./releasing.md) — the release checklist (the only checklist; `AGENTS.md` points here)
- [packaging.md](./packaging.md) — Rust binary packaging + tester first-run
- [updates.md](./updates.md) — manual update check (no background pings)
- [tray.md](./tray.md) — system tray / menu bar
- [design-system.md](./design-system.md) — design tokens + messaging (app + site)
- [sample-dataset.md](./sample-dataset.md) — `sample/` demo corpus guide
- [sample-ai.md](./sample-ai.md) — `sample-ai/` demo corpus guide
- [bench-chunking.md](./bench-chunking.md) — chunking/retrieval benchmark harness (Phase 36)
- [guides/hierarchical-toc.md](./guides/hierarchical-toc.md) — hierarchical TOC + tag taxonomy (Phase 14)
- [guides/rest-api.md](./guides/rest-api.md) — connect any JSON list API as a source
- [guides/obsidian-vault.md](./guides/obsidian-vault.md) — point AnchorCore at an Obsidian vault

## Plans (designs + proposals — check `rust/BACKLOG.md` for what landed)

- [product-plan.md](./product-plan.md) — product scope + backlog narrative (canonical for scope)
- [v2v3-scope.md](./v2v3-scope.md) — v2/v3 business + platform scoping (partly executed)
- [hosted-cloud-plan.md](./hosted-cloud-plan.md) — future hosted cloud tier plan, not operations (Team ops: `hosting/README.md`)
- [gated-access-spec.md](./gated-access-spec.md) — deterministic gated reads + team consumer gating (Part A shipped reduced; body is design history)
- [plan-backup-sync.md](./plan-backup-sync.md) — backup/export/encrypted sync plan (Phase A shipped 2026-10-12; B/C gated on §8 sign-off)
- [plan-mcp-zero-setup.md](./plan-mcp-zero-setup.md) — MCP zero-setup UX plan (R20.4 + R20.10 open)

## Historical (snapshots — read for context, do not update)

All live in [`archive/`](./archive/) — new session handovers go straight there too.

- `archive/handover-*.md` — session handovers (dated)
- `archive/port-review-2026-08-21*.md`, [review-2026-08-21-full.md](./archive/review-2026-08-21-full.md) — port reviews
- [archive/rust-port.md](./archive/rust-port.md) — Rust port evaluation (shipped v1.0.9; historical)
- [archive/plan-code-sections.md](./archive/plan-code-sections.md) — code-aware sections (shipped 2026-10-12)
- [archive/plan-code-retrieval.md](./archive/plan-code-retrieval.md) — code indexing redesign (phase complete 2026-10-14)
- [archive/plan-entities-ux.md](./archive/plan-entities-ux.md) — Entities + Review audit + proposal (landed 2026-10-14)
- [archive/audit-r33.6-kinds.md](./archive/audit-r33.6-kinds.md) — entity-kind audit (R33.6 landed)
- [archived-python.md](./archived-python.md) — what froze on tag `archive/python-final`
- [b48-clean-machine.md](./b48-clean-machine.md) — B48 clean-machine test script (v1.0.13)
- [archive/independent-code-review.md](./archive/independent-code-review.md) — independent code review (2026-10-13)
- [archive/pilot-retro-2026-10-09-b54.md](./archive/pilot-retro-2026-10-09-b54.md) — B54 agent-build pilot retro

## Release notes

- [release-notes/](./release-notes/) — per-version user-facing notes (`v1.0.22.md` onward; older tags have no notes — do not backfill from memory)
