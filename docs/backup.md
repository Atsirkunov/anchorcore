# Backup, export & device migration

> Phase A of `docs/plan-backup-sync.md` (product plan B61). One portable archive
> format, one engine, three callers: the UI, the CLI, and (later) the Team image.
> This document is the user-facing guide **and** the format spec the importer
> and the future sync protocol translate.

- [What an export is](#what-an-export-is)
- [What moves, what does not](#what-moves-what-does-not)
- [Exporting](#exporting)
- [Importing](#importing)
- [Secrets model](#secrets-model)
- [`.anchorcore-backup` format spec](#anchorcore-backup-format-spec)
- [The raw snapshot (advanced)](#the-raw-snapshot-advanced)
- [Acceptance scripts](#acceptance-scripts)

## What an export is

`anchorcore-backup-<timestamp>-<rand>.anchorcore-backup` is a zip file — safe to
put on a USB stick or in any cloud drive. It contains your **memory**: sources,
documents, extracted entities, chunks/sections, the tag taxonomy, projects, and
the review state you built (verified/disputed entities, merges, PII decisions).
It does **not** contain your source credentials unless you explicitly opt in
with a passphrase, and it never contains anything in plaintext that you would
not find in the app itself.

Typical sizes: deflate compresses document text ~4–6×, and embeddings are
excluded by default (they are recomputed on the new device), so a
5,000-document corpus exports in tens of megabytes.

## What moves, what does not

| Item | Moved? | Notes |
|---|---|---|
| sources (connector, name, label, non-secret config) | yes | `needs_secret` lists credentials to re-enter |
| ingested items (title, text, hash, doc type, stale) | yes | documents are re-imported, not re-fetched |
| entities, relationships, disputes, merge actions | yes | review state is the point of a real backup |
| chunks, sections, tags, `chunk_tags` | yes | `uid`-referenced, no local ids |
| projects + project→source links | yes | scoped export with `project_id` is supported |
| non-secret settings (`app_settings`) | yes | provider API keys never leave |
| embeddings | opt-in | `include_embeddings`; records `embed_model` for mismatch detection |
| source credentials | opt-in | `include_secrets` + passphrase → `secrets.bundle` |
| jobs, system events, users | opt-in | `include_ops` |
| folder source paths | remap hint | `manifest.json` lists them; machine paths differ |

## Exporting

`POST /backup/export` (JSON body) with:

| field | default | meaning |
|---|---|---|
| `include_secrets` | `false` | add `secrets.bundle`; requires `passphrase` |
| `passphrase` | — | passphrase for the bundle (never stored) |
| `include_embeddings` | `false` | include vectors (same embed model only) |
| `include_ops` | `false` | include jobs / system events / users |
| `project_id` | all | restrict the export to one project's sources |

The response streams the archive with
`Content-Disposition: attachment`. The same options drive the System tab card
and `anchorcore export` (CLI), so all three callers produce the same file.

In the app: **System → Backup & restore → Export archive**. The passphrase
field only appears when you tick "Include source credentials", and the button
stays disabled until you provide one.

## CLI (headless / Team)

```bash
# export (passphrase, when used, comes from an environment variable — never argv)
ANCHOR_BACKUP_PASSPHRASE='...' anchorcore export --out ~/backups/ac.anchorcore-backup --include-secrets

# inspect an archive before touching the local data
anchorcore import --file ac.anchorcore-backup --dry-run

# merge it (uid upsert, local newer state wins)
anchorcore import --file ac.anchorcore-backup

# replace everything (typed confirmation required; safety backup first)
anchorcore import --file ac.anchorcore-backup --mode replace --yes-destructive
```

`--data-dir`, `--port` and `--host` are global flags and work before or after
the subcommand. When an AnchorCore instance is reachable
(`ANCHOR_BACKEND_URL`, else `http://127.0.0.1:<port>`, else `127.0.0.1:8000`)
the CLI talks to its API, so the app stays the single owner of the database;
otherwise it works on the database directly (app stopped) and a locked
database tells you to stop the app or point the CLI at its API. `--remap
source_uid=new_path` (repeatable) remaps folder sources while importing.

Exports are consistent while the app runs: the archive is written inside a
single read snapshot (SQLite WAL), so a sync running in parallel cannot produce
a half-updated record set. Every export writes a `backup` audit event into
`system_events`.

Finished archives live in `<data dir>/exports/` only for as long as the download
needs them; older files are swept on the next export.

## Importing

`POST /backup/import` is multipart: `file` (the archive) plus an optional
`options` JSON field. Without `options` the call is a merge **dry run**.

| option | default | meaning |
|---|---|---|
| `mode` | `merge` | `merge` (uid upsert, last-writer-wins per record) or `replace` |
| `dry_run` | `false` | report what would happen; nothing is written |
| `confirm` | — | `replace` requires the exact string `replace` |
| `passphrase` | — | opens `secrets.bundle` when present; without one the bundle is skipped with a note |
| `remap` | — | `{source_uid: new folder path}` applied to folder sources |

The report contains: per-type counts, imported/skipped per type, conflicts (up
to 50, each marked `imported` or `kept-local`), `needs_secret` and folder paths
from the manifest, applied remaps, `secrets_imported`, `safety_backup` (replace
only), `reembed_source_ids`/`reembed_job_ids`, and warnings.

Semantics:

- **merge** matches records by uid. An existing record is updated only when the
  incoming `updated_at`/`created_at` is newer (lexicographic timestamp LWW;
  Phase B replaces this with a hybrid logical clock) — otherwise the local copy
  wins and the record is reported as a conflict.
- **replace** needs the typed confirmation, takes an automatic safety backup
  (`exports/safety-backup-*.anchorcore-backup`, which the sweep never deletes),
  then deletes all data records inside the same transaction. Settings, ops data
  and users stay local.
- **settings** that already exist on this machine win (provider config is
  machine-local, plan §2); secrets never travel through settings.
- **embeddings** from another model (dimension mismatch) are dropped with a
  warning and recomputed, never silently indexed.
- **re-embed**: sources whose chunks still lack vectors are queued as `embed`
  jobs (`jobs.kind='embed'`) that run through the same gated embedder as
  ingestion; retrieval reports keyword-only until they finish. Tag vectors are
  local and rebuilt inline.
- The whole import is **one transaction** — an error rolls everything back.
- Validation: `format_version` gate ("upgrade the app"), entry-name checks
  (zip-slip), decompression caps plus a compression-ratio bomb guard, and the
  bundle's KDF caps.

A dry run applies the records inside the transaction and rolls back, so its
conflict report is exactly what a real import would do.

In the app: **System → Backup & restore → Import**. Pick the archive, run
**Preview (dry run)** to see the counts, the credentials to re-enter and the
folder paths to confirm, then **Import (merge)**. Replacing everything needs
the typed `replace` confirmation; the safety backup name is shown in the
result.

After import, chunks without vectors are queued for embedding; until that
finishes, retrieval honestly reports keyword-only.

## Secrets model

- **Default: no secrets.** An export without `include_secrets` contains only
  non-secret config plus a `needs_secret` list in the manifest, so a stolen
  backup file leaks no credentials. Tokens rotate anyway — re-entering them is
  the safe default.
- **Opt-in bundle.** `include_secrets` + a passphrase writes `secrets.bundle`:
  Argon2id (m = 64 MiB, t = 3, p = 1) → 32-byte key, XChaCha20-Poly1305 AEAD,
  random 16-byte salt and 24-byte nonce per file. The whole header is bound as
  associated data, so tampering with KDF parameters, salt or nonce fails
  decryption instead of silently changing the key. The key and plaintext are
  zeroized after use.
- **No recovery.** There is no escrow, no hint, no reset. Losing the
  passphrase means the bundle is unrecoverable — re-enter the credentials
  instead. This is stated in the archive's `README.txt` too.
- Secrets are keyed by **source uid** (`source:<uid>:<field>`), never by the
  local integer id, so an import can remap them to a new device.

## `.anchorcore-backup` format spec

```
manifest.json     format_version, app_version, platform, created_at, device,
                  counts, embed_model/embed_dim, options used, needs_secret,
                  folder_paths
data.jsonl        one typed JSON record per line (deflate-compressed in the zip)
secrets.bundle    optional; only with include_secrets + passphrase
README.txt        what this is, how to restore, keep-your-passphrase warning
```

`format_version` is versioned **independently of the SQLite schema** and starts
at `1`. The importer accepts the versions it knows and refuses anything newer
with an "upgrade the app" message; when the record schema changes, bump the
version and keep a reader for the previous one. Records never carry local
integer ids — only uids (UUIDv7) — so ids can be remapped on import.

Record types (`data.jsonl`):

| `type` | carries |
|---|---|
| `source` | `uid`, connector, name, label, enabled, non-secret `config`, `needs_secret`, `created_at` |
| `item` | `uid`, `source_uid`, external_id, title, doc_type, text, content_hash, author, updated_at, stale |
| `entity` | `uid`, `item_uid`, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, dispute_count, timestamps |
| `relationship` | `from_uid`, `to_uid`, kind, confidence, source_ref |
| `chunk` | `uid`, `item_uid`/`entity_uid`/`section_uid`, kind, source_ref, content, level, path, is_pii, pii_categories |
| `chunk_embedding` / `section_embedding` / `tag_embedding` | `uid` + `embedding_b64` — only with `include_embeddings` |
| `section` | `uid`, `item_uid`, `parent_uid`, level, title, path, chunk_range, summary, summary_hash |
| `tag` / `chunk_tag` | name, description, count / `chunk_uid` + `tag_uid` |
| `project` | `uid`, name, is_default, `source_uids` |
| `dispute` / `merge_action` | `entity_uid`(s), reason/status/user, timestamps |
| `setting` | key, value — **secret keys and empty values are never exported** |
| `ops_jobs` / `ops_system_events` / `ops_users` | raw rows, only with `include_ops` |

`secrets.bundle` layout:

```
"ACSB1\n"          6 bytes   magic
u32 (LE)           header length
header (JSON)      v, kdf:"argon2id", m_cost, t_cost, p_cost, salt, nonce (base64)
ciphertext         XChaCha20-Poly1305(payload), AAD = magic + length + header
payload (JSON)     {"v":1,"entries":[{"k":"source:<uid>:token","v":"..."}]}
```

The importer caps the KDF parameters a bundle may request (m ≤ 64 MiB, t ≤ 16,
p ≤ 8) so a hostile archive cannot turn into a memory bomb.

## The raw snapshot (advanced)

`VACUUM INTO` a copy of `anchorcore.db` and zip it. That is a byte-faithful
escape hatch for support and debugging — same app version only, no secrets, no
translation. **It is not the migration path:** schema churn, the Postgres swap
(B49) and device sync all need record-level translation, and JSONL survives
partial tooling. Use the portable archive for moving devices.

## Acceptance scripts

Run each on Windows **and** macOS. These are the Phase A gates from the plan
(§10 A1–A3).

**A1 — device migration.** Machine A: add the sample corpus, verify one entity,
dispute another, sync. Export without secrets. Machine B: fresh install →
Import → the same 10 sample questions answer with equal citations, verify/
dispute state intact → re-enter one source token → sync works. No repo, no
Rust, no manual DB work.

**A2 — restore after loss.** Delete the data dir; import the last backup; ask;
answers + review state restored; the credentials list matches the manifest.

**A3 — secrets bundle.** Export with a passphrase; on a clean machine import
with that passphrase → `GET /sources/{id}/config` shows `***set***` and a sync
succeeds; a wrong passphrase fails cleanly with no partial import.

A4/A5 (folder packs, zero-knowledge relay) belong to Phases B/C and stay in
`docs/plan-backup-sync.md` §9 until Phase A ships and the §8 decisions are
signed off.
