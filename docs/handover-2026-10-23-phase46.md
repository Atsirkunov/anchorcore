# Handover 2026-10-23: Phase 46 (flows + images), 9 of 15 rows shipped

For the next agent / owner. Phase plan + owner decisions: `rust/BACKLOG.md`
Phase 46 and `rust/docs/decisions.md` (2026-10-23 entries: Phase 46, R46.7,
R46.9, R46.5).

## 1. State of the tree

- `main` at `67df27a`, pushed, working tree clean. Not released: still
  1.3.0 in `rust/Cargo.toml`, no tag.
- Gates on `67df27a`: `cargo test -p anchorcore` 563 lib + all `tests/api_*`
  + golden eval green (`-D warnings`), lizard no new warnings, frontend
  build + vitest 85/85.
- Also on `main` today (other sessions): R30.1, R30.7, CI docs-only skip,
  and `41001c9`, the lib-target extraction (modules live in `src/lib.rs`,
  routes in `src/routes.rs`, `main.rs` is boot only; unit tests run under
  `--lib`, the eval no longer re-runs ~100 unit tests).
- Fixed in passing: `SourceLabels.test.tsx` had been red since R30.1
  (missing `QueryClientProvider`), so CI's `npm run test` step was failing.

## 2. What shipped (Phase 46)

| Row | Commit | Summary |
|---|---|---|
| R46.1 | `47a229e` | `diagram.rs` normal form → markdown (frames, reading order, `A → B: label`) |
| R46.2 | `23861d2` | draw.io / Excalidraw / SVG via `diagram_formats.rs`; `extract::diagram_kind` |
| R46.3 | `4814bb8` | Visio `.vsdx`/`.vsdm` |
| R46.5 | `67df27a` | Miro connector (`connectors/miro.rs`, `docs/connectors/miro.md`) |
| R46.7 | `e5d6031` | migration 26 assets + blobs in the DB; embedded + file images; backup `assets/<sha256>` |
| R46.8 | `e0330df` | docx/pptx/ODF images as `embedded:` markers; pptx slides are `# Slide N` |
| R46.9 | `3e46f03` | `GET /media/:uid`; `images` on hits (opt-in) + agent citations; item `assets` |
| R46.10 | `47c3f5f` | MCP `get_asset`; REST `?format=json` with the same 1.5 MB cap + gate |
| R46.11 | `4b5e8d7` | UI thumbnails (`ImageStrip.tsx`) on source cards + Entities "Show document" |

## 3. Contracts to keep (also in AGENTS.md)

- Image bytes live only in the DB (`asset_blobs`, sha256 key, GC triggers).
  Never path references or fetch-later; export must restore every image with
  the source gone.
- Assets are served at `/media/*`, never `/assets/*`. That prefix is the public
  SPA bundle dir in `auth::is_public_path`.
- Sync never overwrites `manual_*` or `manual`/`ignored` attribution; the asset
  uid is deterministic (item uid + src + ordinal).
- No network on ingest: remote image URLs are skipped (alt stays text).
- Lizard: an apostrophe in a `///` comment or a `'"'` char literal in
  `extract.rs` mis-scopes spans. `resolve_ref` is already mis-scoped on the
  baseline and sits just under the length threshold.

## 4. Known gaps

- Office images (R46.8) only resolve for folder sources; Drive keeps no package
  bytes → **R46.15**.
- Miro citations link the board, not the item; deleting a source's only
  board doesn't purge its item (existing empty-fetch guard).
- R46.11 thumbnails were never looked at in a browser. Check light + dark,
  lock tiles, no layout shift.
- `asset_max_bytes` (20 MB) is provisional and API/env-only (no Settings field).

## 5. Resume here

1. **R46.12** "Unplaced images" (P2, 1.5d): routes under `/media` (the
   backlog row already says so), people-only captions, test asserts no chat
   call.
2. **R46.15** Drive Office images (P2, 0.5d). Shares `pipeline.rs`/`assets.rs`
   with R46.12: one worker each, merge after.
3. **R46.14** size visibility + final cap: needs numbers from a real folder
   corpus and a Drive or Miro corpus (owner).
4. P3 on demand: R46.4 (Mermaid fences), R46.6 (FigJam/Lucid), R46.13
   (Miro images).
5. A release (1.4.0) bundling Phase 46 once R46.12 lands; follow `docs/releasing.md`.
