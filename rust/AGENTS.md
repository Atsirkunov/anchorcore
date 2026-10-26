# AnchorCore Rust — workspace notes

Root [`AGENTS.md`](../AGENTS.md) is the guide (run/test/release, architecture map, gotchas). This file
holds only what is specific to working inside `rust/`.

## Layout

- `crates/anchorcore/` — the one binary: `src/` (modules mapped in root `AGENTS.md`), `migrations/*.sql`
  (registered in `db.rs`), `tests/api_*.rs` (black-box HTTP contract tests — each spawns the binary on a
  free port with a fresh data dir), `tests/retrieval_eval.rs` + `tests/eval_data/` (golden eval).
- `BACKLOG.md` (open work) · `BACKLOG-archive.md` (shipped/closed) · `docs/decisions.md` (ADR log).
- `scripts/` — bench + version helpers (`sync_version.py` is a duplicate of the root copy; CI watches drift).

## Complexity gate

- **Every function ≤ 15 CCN** (lizard). When a handler or orchestrator passes ~12, split it into named
  phase helpers (`pipeline.rs` `ClassifyCtx` phases, `system.rs` `status_snapshot`) and prefer guard
  clauses (`let Some(x) = y else { return … }`) over nested `if let`/`match`.
- **Gate:** `lizard -l rust -w rust/crates/anchorcore/src -C 15` prints no new warnings. Accepted baseline
  (7, re-measured 2026-10-26): `rtf_to_text` 31 (R36.14), `create_summaries_inner` 32,
  `create_tags_inner` 19, `conflicts_for_chunks` / `load_hits_filtered` /
  `load_chunks_with_embedding_filtered` 17, and the `text_decodes_utf8_utf16_and_cp1252` test-span
  tokenizer artifact. If lizard flags a function your diff never touched, suspect the tokenizer (root
  `AGENTS.md` gotcha) before believing it.
- Reusable patterns: `with_db` / `run_db` for all blocking DB work; row-mapper helper fns instead of
  inline closures; `params_from_iter` vecs instead of repeated match arms.
