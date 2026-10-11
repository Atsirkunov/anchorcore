# Handover — Phase 25 (backup / export / device migration)

Written 2026-10-11 by the session that did the Phase 18 website audit. Phase 25 is
**not started on purpose**: this box cannot link Rust (no C toolchain — `cargo
check`/`cargo test` die in dependency build scripts), and Phase 25 is the one
phase where guessing at migrations through 5-minute CI cycles is expensive. Run
it from a machine with a working toolchain.

## Read first, in this order

1. `docs/plan-backup-sync.md` — the plan of record.
   - §3.1 the `.anchorcore-backup` archive (`manifest.json`, `data.jsonl`,
     `secrets.bundle`, `README.txt`) and the record/identity table
   - §3.2 the raw `VACUUM INTO` snapshot (support escape hatch, not the migration path)
   - §4 the Phase A API (`POST /backup/export|import`, dry-run, replace semantics)
   - §7 crypto + **the mandatory grep test**
   - §8 the decisions — all "recommended" options are the intended ones
   - §10 acceptance scripts A1–A3 (A4/A5 are Phases B/C, not this work)
2. `rust/BACKLOG.md` → Phase 25 (R25.1–R25.7): per-task DoD, estimates, files.
3. `AGENTS.md` (root) and `rust/AGENTS.md` — the contract-first workflow and the
   ≤15 CCN rule are gates, not suggestions.

## Before writing code

- **Use a fresh clone or `git worktree add ../ac-phase25`.** Another session was
  committing in this working tree at handover time (see "Repo state" below). Do
  not `git add -A` in a shared tree — `AGENTS.md` documents how 896 vendored
  skill files got committed that way once.
- Needs a linker: `cargo test -p anchorcore` and `cargo check` must run locally.

## Order (the dependencies are real)

R25.1 (uids) + R25.6 (secrets bundle) in parallel → R25.2 (export) → R25.3
(import) → R25.4 (CLI) + R25.5 (UI) in parallel → R25.7 (docs + acceptance).
~5 days total by the plan's sizing.

## Facts that would otherwise take an hour to re-derive

- **Migrations.** `rust/crates/anchorcore/migrations/` holds `01…13`; the next is
  **`14_uids.sql`** (as R25.1 specifies), and it must also be appended to the vec
  in `db.rs:migrations()` (function at `db.rs:249`). `PRAGMA foreign_keys=ON`
  stays — source deletion depends on it.
- **`uid` coverage** (R25.1): `sources`, `ingested_items`, `entities`, `chunks`,
  `sections`, `tags`, `projects`, plus backfill for existing rows. Disputes and
  merge actions reference uids. API output stays unchanged (uid is internal until
  Phase B).
- **Deps already in `rust/crates/anchorcore/Cargo.toml` and reusable:** `zip` 9
  (deflate), `sha2`, `hex`, `base64`, `rand` 0.8, `ed25519-dalek`, `serde` +
  `serde_json`, `chrono`, `clap` (for R25.4), `tempfile` (dev).
- **Deps to add:** `argon2` (Argon2id), `chacha20poly1305` (XChaCha20-Poly1305),
  `uuid` with the `v7` feature, `zeroize`, and `axum`'s `multipart` feature for
  the import endpoint.
- **Conventions that bite:** all DB work goes through `spawn_blocking` and the
  `with_db`/`run_db` helpers in `pipeline.rs`; never hold a `Connection` across
  `await`; secrets are never plaintext and never returned to the UI
  (`GET /sources/{id}/config` masks them as `***set***`); `secrets.enc.<sha256>`
  per-key files are the fallback store.
- **Honest degradation:** an import that skips embeddings must leave retrieval
  reporting `keyword_only` (B58) instead of pretending. The export excludes
  embeddings by default and the manifest records `embed_model` so a mismatch is
  detectable rather than silent.
- **`format_version`** is versioned independently of SQLite; refuse a *newer*
  archive than the running app with an "upgrade the app" message, and document
  the policy in `docs/backup.md` (R25.7).

## Gates before pushing

- `cargo test -p anchorcore` green, `RUSTFLAGS="-D warnings" cargo check` clean,
  `cargo clippy -p anchorcore -- -D clippy::unwrap_used` (local `allow`s exist).
- `lizard -l rust -w rust/crates/anchorcore/src -C 15` — no new warnings beyond
  the accepted `is_public_path` decision table.
- Conformance: `rust/scripts/conformance.sh` (it restores `backend/` from tag
  `archive/python-final`), or `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=… pytest -q`.
- R25.5's UI half: `cd frontend && npm run build && npm run lint && npm run test`.
- **The plan's grep test (§7):** plant a marker token + marker phrase, export with
  and without `include_secrets`, assert neither appears in the plaintext bytes of
  any output. R25.2's DoD makes this a test, not a manual check.
- **Land it on `main`**, one commit per task with an `R25.x:` prefix and the
  evidence in the message — the Phase 18 commits (`git log --oneline` around
  `c5a096b…81651eb`) are the house style: what changed, what was measured, and
  what was deliberately not done.
- **Do not tag a release.** The owner controls releases (a tag also deploys the
  website, and they batch it).

## What "done" looks like

R25.1–R25.7 shipped with their DoD met and the Phase 25 table reduced to zero
open rows; `docs/backup.md` lets a non-developer run acceptance scripts A1–A3 on
Windows and macOS; the raw-snapshot path is documented as advanced-only. Phase B
(folder packs) is gated on a 2-device walkthrough of Phase A, so leave
`docs/plan-backup-sync.md` §9's gate note intact.

## Repo state at handover

- `main` == `origin/main` at `a32e350`; **v1.0.19** is the shipped version and the
  website is deployed. Phase 18: 18 shipped / 1 partial (deferred by the owner).
  Phase 26 (website tooling): 2 shipped. Open work otherwise: Phase 20 (4) and
  Phase 23 (2).
- Not yet live (no tag since v1.0.19): R26.1's website type-check/lint gate,
  R26.2's `--stone` contrast fix, and R18.19's agent-skill entry point.
- **A second session was active in this working tree** — its commits `d6ab93f`
  (896 vendored skill files out of the index), `a32e350` (README note) and
  `fabd55a` (R18.19), plus an uncommitted `.gitignore` edit at handover time.
  That is why this file says to work in your own clone.
