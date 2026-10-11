# Handover — Phase 25 (backup / export / device migration) shipped

Written 2026-10-12 by the session that implemented Phase 25. Phase A of
`docs/plan-backup-sync.md` is **done on a branch** (`r25-phase-a` in the
`../ac-phase25` worktree, based on `main` at `ed0853b` = `0a4e888` + the other
session's R20.6/R20.7): R25.1–R25.7, one commit per task, `cargo test` 175/175,
lint/build/tests green, and live Windows runs of the API, the CLI and the UI.

## What shipped

| Task | What | Where |
|---|---|---|
| R25.1 | UUIDv7 `uid` on the 7 syncable tables (+ backfill, insert paths) | `migrations/14_uids.sql`, `db.rs` |
| R25.6 | Argon2id (64 MiB, t=3, p=1) + XChaCha20-Poly1305 secrets bundle, header bound as AAD | `backup.rs`, `docs/backup.md` |
| R25.2 | `.anchorcore-backup` export (`POST /backup/export`), records via SQLite `json_object`, one WAL snapshot, streamed download | `backup.rs`, `main.rs` |
| R25.3 | Import (`POST /backup/import`, multipart): dry-run report, merge (uid + timestamp LWW), replace (typed confirm + never-swept safety backup), zip-slip/bomb/KDF caps, re-embed jobs | `backup.rs`, `pipeline.rs` |
| R25.4 | CLI `anchorcore export|import` — prefers a running API, else direct DB with a locked-DB hint | `cli.rs`, `main.rs` |
| R25.5 | System tab "Backup & restore" card (export options, dry-run report, remap inputs, credentials list, replace confirm, re-embed progress) | `frontend/src/BackupCard.tsx` |
| R25.7 | `docs/backup.md` (format spec + walkthrough + verified table), README link, B61 status, AGENTS notes | docs |

## Evidence

- `cargo +stable-x86_64-pc-windows-gnu test -p anchorcore --target x86_64-pc-windows-gnu` → **175/175**
  (11 backup tests incl. the plan §7 grep test, 6 import tests, 4 CLI tests, 3 UI component tests).
- Live runs (Windows, debug binary, fresh data dirs):
  - seed 10 sample docs → 35 entities → export (20 KB) → import into an empty dir →
    `/qa` returns `retrieval=hybrid` with citations, `embed` job 56/56 done;
  - replace into an existing dir → typed confirm required, `safety-backup-*` left on disk;
  - wrong passphrase → error and **0 sources** imported; right passphrase → `secrets_imported=1`
    and `GET /sources/1/config` shows `***set***`; the raw token never appears in the archive bytes;
  - `format_version: 99` refused with "upgrade the app";
  - CLI against the running API: `Using the running AnchorCore at http://127.0.0.1:8234` + archive;
  - Playwright: 10/10 checks on the System-tab card, zero console errors.
- `RUSTFLAGS="-D warnings" cargo check` clean; `lizard -C 15` adds no warnings; clippy adds none for
  `backup.rs`/`cli.rs`/`tags.rs` (two pre-existing hits in `pipeline.rs`).

## Still open (honest gaps)

1. **macOS leg of A1–A3** and a two-machine run from the release zip — no Mac in this session; the
   format is OS-independent, so this is a verification gap, not a known defect.
2. `jobs.kind='export'` for very large corpora (the plan's async export job) is deliberately deferred:
   exports stream from a temp file in `exports/` and are swept after an hour.
3. Phase B/C (folder packs, zero-knowledge relay) stay gated on the plan §8 sign-off — do not start
   them from `docs/plan-backup-sync.md` §9 without the owner.

## Notes for the next agent

- This box **can** build Rust: WinLibs MinGW on PATH + `cargo +stable-x86_64-pc-windows-gnu
  --target x86_64-pc-windows-gnu` from `rust/` (the old "no C toolchain" note in the docs was stale
  and has been updated). A fresh worktree needs `rust/target` and `frontend/dist` robocopied or rebuilt.
- The worktree was needed because the main checkout had another session committing at handover time;
  `r25-phase-a` is based on `ed0853b` and needs a rebase/merge before landing on `main`.
- Live-test rigs (stub servers, Playwright script, archives) live in `%TEMP%\opencode` — TEMP, may not
  survive a reboot; the commands are in this doc and in the commit messages.
