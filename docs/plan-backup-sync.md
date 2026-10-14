# AnchorCore — Backup, export & encrypted device sync (plan)

> Status: proposed 2026-10-11 — decisions §8 need owner sign-off before Phase A
> starts; Phases B/C are design-only until then. Phase A tasks:
> `rust/BACKLOG.md` Phase 25 (R25.1–R25.7). Product anchor and priority:
> `docs/product-plan.md` B61.
>
> Trigger: "plan and design backup / export feature in case user changes the
> devices… future hosted solution where we will allow users to sync information
> between devices (while ideally keeping the privacy component as much as we
> can in this setup)".

## 1. Problem

AnchorCore stores everything the user owns in one place with no way out:
`~/.anchorcore/anchorcore.db` plus OS-keychain secrets. Today "move to a new
laptop" means re-adding every source, re-entering every token, and re-syncing
from scratch — and a disk failure loses all reviewed memory (verified entities,
disputes, merges, PII decisions), which re-syncing cannot recreate.

Goals, in order:

1. **Device migration** without re-work (the trigger).
2. **Backup / restore** that protects review state, not just raw documents.
3. **Portability** across app versions and operating systems — not a SQLite
   file copy.
4. **A privacy-preserving path to multi-device sync** under the future hosted
   tier: the server must be *structurally* unable to read user content.
5. **GDPR Art. 20 portability** for the hosted tier — `hosted-cloud-plan.md` (Security posture) already
   lists export/delete as a blocker before the first EU customer.

Non-goals (explicit): real-time collaboration, per-user ACL sync (see
`gated-access-spec.md` Part B), syncing *source content* (documents are
re-fetched from their connectors; this plan moves **memory**, not files).

## 2. What actually moves (inventory)

`resolve_data_dir` (`main.rs:66`) → `~/.anchorcore` by default. Contents and
portability:

| Item | Where | Portable? | Decision |
|---|---|---|---|
| `anchorcore.db` — sources, `ingested_items`, entities, relationships, chunks, sections, tags, `chunk_tags`, projects, `project_sources`, `disputes`, `merge_actions`, `app_settings` | data dir | yes, record by record | **core payload** |
| Secrets — source tokens, API keys (`token`/`api_key`/`password`/`client_secret`) | OS keychain; fallback `secrets.enc.<sha256>` + `secrets.key` | **no** | never plaintext; re-enter by default, optional passphrase bundle |
| Embeddings (`chunks.embedding`, `sections.summary_embedding`, `vec_chunks`) | DB | derived, **content-bearing** | recompute locally; exclude by default |
| `chunks_fts` | DB | derived | rebuilt from `chunks` triggers |
| `jobs`, `system_events`, `users`, `anchorcore.log` | DB / dir | ops data | excluded by default (`include_ops` flag) |
| Machine settings — provider base URLs, model names, timeouts | `app_settings` | machine-local | import keeps the current machine's provider config by default |
| Folder source `path`s, `source_ref` prefixes (`\\?\…` on Windows, B53) | config / refs | machine-local | path-remap on import; refs stay as provenance |

Two facts drive the whole design:

- **"Zip the folder" is not a backup.** With a working OS keychain the secrets
  are not in the folder at all; with `ANCHOR_SECRETS_NO_KEYRING=1` the Fernet
  key (`secrets.key`, `secrets.rs:25-45`) sits *next to* the ciphertext, so the
  copy is plaintext-equivalent. Export must own secrets explicitly.
- **IDs are per-device integers.** Two devices both have entity `id=1`.
  Multi-device anything needs stable ids → UUIDv7 `uid` columns (Phase A, §6).

## 3. Artifacts — one schema, two files

### 3.1 Primary: `.anchorcore-backup` (portable archive)

A zip, safe to store anywhere (no plaintext secrets unless opted in):

```
manifest.json     # format_version, app_version, platform, created_at,
                  # device label, counts, embed_model, options used
data.jsonl        # one typed JSON record per line, deflate-compressed inside the zip
secrets.bundle    # optional — only with include_secrets + passphrase
README.txt        # what this is, how to restore, keep-the-passphrase warning
```

Record types — an **export schema versioned independently of SQLite** (this is
the contract the importer, and later the sync protocol, translate):

| record | carries | identity |
|---|---|---|
| `source` | connector, name, label, enabled, non-secret config, `needs_secret:[…]` | uid; fallback (connector, name) |
| `item` | source ref, external_id, title, doc_type, `text`, content_hash, stale | uid; fallback (source, external_id) / content_hash |
| `entity` | item ref, kind, summary, reasoning, confidence, author, source_ref, window_index, window_text, status, owner, timestamps | uid; fallback (item, window_index, kind) |
| `relationship` | from/to entity refs, kind, confidence, source_ref | uid pair |
| `chunk` | item ref, kind, source_ref, content, path, level, is_pii, pii_categories, section path | uid |
| `section` | item ref, parent path, level, title, path, summary | uid |
| `tag` / `chunk_tag` | name, description / chunk ref | uid / pair |
| `project` | name, is_default, source refs | uid |
| `dispute`, `merge_action` | entity refs, reason/status/user, timestamps | uid refs |
| `setting` | key, value (non-secret; provider keys excluded) | key |

Embeddings are not in the format by default. `include_embeddings` exists for
same-model restores and records `embed_model` in the manifest so a mismatch is
detected — never sent anywhere without encryption.

### 3.2 Secondary: raw snapshot (power users / support)

`VACUUM INTO '<tmp>'` copy of `anchorcore.db` (consistent while the app runs,
SQLite ≥ 3.27) zipped. Same-version restore only, no secrets, clearly labeled
advanced — a byte-faithful escape hatch for support and debugging, not the
migration path. Reasoning: schema churn (13 migrations to date), the Postgres
swap (B49), and sync all need record-level translation anyway; JSONL also
survives partial tooling.

## 4. Export / import — Phase A (local, no server)

One engine, three callers (UI, CLI, Team). Proposed API:

- `POST /backup/export` `{include_secrets, passphrase?, include_embeddings,
  include_ops, project_id?}` → streams the archive
  (`Content-Disposition`); long exports become a job
  (`jobs.kind='export'`) with progress reuse.
- `POST /backup/import` (multipart) `{mode: merge|replace, dry_run}` →
  dry-run report: record counts, `format_version` gate, per-source
  `needs_secret`, folder paths that need remapping, conflicts. `replace`
  requires typed confirmation and takes an automatic safety backup first.
- Import is transactional; entries are validated (zip-slip, decompression
  caps); a newer `format_version` than the running app is refused with a
  "upgrade the app" message.

Secrets: excluded by default — the manifest lists what to re-enter, the import
UI shows "enter credentials for 3 sources". `include_secrets` requires a
passphrase and writes `secrets.bundle` (Argon2id + XChaCha20-Poly1305, §7).
Re-embed on import: enqueue the embed job for chunks missing vectors; retrieval
degrades to `keyword_only` honestly (B58) until it finishes.

UI: System tab "Backup & restore" card — Export (options + passphrase), Import
(file → report → confirm), credentials list, path-remap inputs.
CLI (headless/Team): `anchorcore export --out …`, `anchorcore import …` — same
engine; talks to the running app (single DB owner), or opens the DB directly
when the app is stopped.

Sizing: the zip's deflate compresses text ~4–6×; embeddings excluded by
default keeps a 5k-document corpus export in the tens of MB.

## 5. Hosted sync — split the word "hosted" first

Today's docs use "hosted" for two different products. Make the split explicit;
it is the privacy decision:

| | **Hosted workspace** (B49 cloud tier) | **Device sync** (this plan) |
|---|---|---|
| Server sees | plaintext — it *runs* the app, models, share links | ciphertext only |
| Privacy | SaaS contract: encryption at rest, tenant isolation, audit | zero-knowledge by structure — operator *cannot* read |
| Features | web access, share links, bundled models, team surfaces | laptop ↔ desktop ↔ Team box sync |
| Trade-off | explicit: you chose the server | none: server is a dumb relay |

The privacy moat lives in **device sync**. The hosted workspace stays a separate
product users can opt into knowingly — this plan does not pretend the cloud tier
can process data without reading it. "AI already knows" (v2v3 pricing table) is
delivered by sync: each device re-embeds locally, so no server model is needed
for the sync story.

## 6. Sync design (Phases B/C)

### 6.1 Identity & keys

- `uid` = UUIDv7 on `sources`, `ingested_items`, `entities`, `chunks`,
  `sections`, `tags`, `projects`; `disputes`/`merge_actions` reference uids.
  Migration lands in Phase A (R25.1) so exports are uid-native from day one —
  no second identity pass later.
- Account key: passphrase → **Argon2id** (m=64 MiB, t=3, p=1) → KEK → unwraps a
  random **account master key**. HKDF-SHA256 derives purpose keys: content
  encryption, index/dedupe, manifest signing.
- Devices: X25519 keypair each; the master key is wrapped per device; the
  server stores salt + wrapped keys + a device registry. Revocation removes the
  envelope and rotates the content-key epoch.
- Recovery: passphrase only, by default none. Setup prints an optional recovery
  code. Losing the passphrase is unrecoverable — stated in the export README,
  the UI, and `docs/backup.md`.

### 6.2 Records & protocol

- Ops, not rows: immutable `{op_id, device_id, hybrid-logical clock, type,
  uid, zstd → XChaCha20-Poly1305 ciphertext}`. The relay stores an op log
  (R2 blobs per workspace) and serves pull-since-cursor / push; periodic
  encrypted snapshots bound replay.
- Merge: per-record last-writer-wins on the HLC, field-level for review state
  (status, owner, merge/PII decisions) so "verify on the laptop, dispute on the
  desktop" doesn't silently lose either; tombstones for deletes (source delete
  cascades).
- Embeddings are **never synced** by default — recomputed per device (privacy:
  embeddings can be inverted toward content; size: ~3 KB/chunk). Optional
  model-fingerprinted embedding packs for large corpora come later.
- What the relay observes: account email (billing), device ids, op counts,
  sizes, timing. Padding/batching/jitter are possible later; documented
  honestly now instead of claimed away.

### 6.3 Transport phasing

- **Phase B — no server: "folder packs."** The same op format written as
  encrypted pack files into a folder the user already syncs (iCloud, Dropbox,
  Syncthing, OneDrive). Proves op log + merge + keys with zero infra; ships
  two-device sync without a paid tier; doubles as the fallback when hosted is
  down.
- **Phase C — hosted / Team relay.** A sync API on the same Axum app (like the
  planned MCP HTTP transport) + R2 per-workspace prefix; the Team container
  gains the same relay behind its license; hosted relay is part of the paid
  tier (v2v3 D4).
- Team caveat: this is **same-user device sync** (and devices of one workspace
  owner). Cross-*user* sync is governed by the Part B ACL work in
  `gated-access-spec.md`, not this plan.

## 7. Threat model & privacy checklist

| Threat | Mitigation |
|---|---|
| Stolen backup file | no plaintext secrets by default; optional bundle is Argon2id + AEAD; embeddings excluded |
| Lost / stolen device | per-device key envelope; revoke device + rotate epoch |
| Honest-but-curious operator | zero-knowledge: ciphertext + minimal metadata only |
| Malicious operator (tamper/replay) | AEAD + signed manifests + monotonic clocks; client rejects |
| Lost passphrase | no silent recovery; recovery code at setup; explicit warnings |
| Metadata analysis | documented gap (counts/sizes/timing); padding/batching as later hardening |
| Shared computer | OS-keychain already gates secrets; export honors the same rule |

Crypto: Argon2id, XChaCha20-Poly1305, HKDF-SHA256, Ed25519 (already a dep),
UUIDv7, `zeroize` for key material.

**The grep test** (mandatory in CI): plant a marker token + marker phrase,
export (with and without `include_secrets`), assert the marker never appears in
plaintext bytes of any output; the same test runs against R2 objects in
Phase C.

Adjacent hardening (separate, cheap): the fallback Fernet key
(`secrets.key`) currently sits next to the ciphertext — derive it from the OS
keystore (DPAPI/Keychain) or a passphrase in a follow-up; flagged here so the
export design doesn't inherit the weakness.

## 8. Decisions needed (recommendations)

1. **Primary artifact** = portable JSONL archive + documented raw snapshot for
   support. *(recommended)*
2. **Secrets default** = excluded, re-enter on import; opt-in passphrase
   bundle. *(recommended — tokens rotate anyway; a lost backup then leaks
   nothing)*
3. **Add UUIDv7 `uid` columns now** rather than inventing natural keys later.
   *(recommended)*
4. **Embeddings** = recompute per device; exclude from exports by default.
   *(recommended)*
5. **Two products, one word each**: "Hosted workspace" (server-side, explicit
   trade-off) vs "Device sync" (zero-knowledge). *(recommended — this is the
   privacy promise)*
6. **Phase B before C**: folder packs prove the sync core without infra.
   *(recommended)*
7. **Team container ships the relay** (license-gated, same user's devices).
   *(recommended)*
8. **Recovery**: passphrase-only + printed recovery code, no server escrow.
   *(recommended; escrow is a support and liability trap)*

## 9. Phasing & task map

| Phase | What | Where | Gate |
|---|---|---|---|
| **A** | uid migration, export/import engine, secrets bundle, CLI, UI card, docs | `rust/BACKLOG.md` Phase 25 (R25.1–R25.7) | this plan accepted |
| **B** | encrypted folder packs (op log, merge, key handling) | new phase after Phase A ships | decisions §8 confirmed by a 2-device walkthrough |
| **C** | hosted/Team relay + device pairing/revocation + R2 | new phase; `docs/hosted-cloud-plan.md` + `v2v3-scope.md` updates | B validated; B49 cloud groundwork |

## 10. Acceptance scripts (run on Windows + macOS)

**A1 — device migration (the trigger).** Machine A: add sample corpus, verify
one entity, dispute one, sync. Export without secrets. Machine B: fresh
release install → import → same 10 sample questions answered with equal
citations, verify/dispute state intact → re-enter one source token → sync
works. No repo, no Rust, no manual DB work.

**A2 — restore after loss.** Delete the data dir; import the last backup; ask;
answers + review state restored; secrets list matches the manifest.

**A3 — secrets bundle.** Export with passphrase; on a clean machine import with
the passphrase → `GET /sources/{id}/config` shows `***set***` and a sync
succeeds; wrong passphrase → clear failure, no partial import.

**A4 — folder sync (Phase B).** Two data dirs, one shared folder: create/verify/
dispute on each side, exchange packs → both converge; concurrent edits resolve
deterministically and are reported.

**A5 — zero-knowledge (Phase C).** Workspace synced to the hosted relay; a
known marker string from the corpus is absent from every R2 object and every
server log; revoking a device stops it from decrypting new epochs.

## 11. Open questions

- Free-tier sync quota (storage/objects) — pricing decision, not architecture.
- Per-project sync granularity later, or whole workspace only? *(recommend
  whole workspace first)*
- Sync `system_events` audit across devices? *(recommend no)*
- Sync account secrets vault (provider keys wrapped by the master key) so a new
  device doesn't re-enter anything? *(later, opt-in — convenient but enlarges
  the blast radius of the passphrase)*
- Chunk-level vs document-level ops for very large corpora — measure with the
  business-scale corpus (B43) before Phase C.
- Does the Team relay server need a "sync off" admin switch for
  regulated environments? *(recommend yes)*

---

*Companion docs: [product-plan.md](./product-plan.md) (B61),
[v2v3-scope.md](./v2v3-scope.md) (§3 hosting, §4 sharing),
[hosted-cloud-plan.md](./hosted-cloud-plan.md) (backups, GDPR), [architecture.md](./architecture.md)
(data model), [gated-access-spec.md](./gated-access-spec.md) (team ACLs).*
