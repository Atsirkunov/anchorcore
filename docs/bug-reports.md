# Bug reports ("it's wrong" flow)

When an answer or search result is wrong, both people and agents generate
the same redacted bundle:

- **People:** the *Report issue* button under any answer in the Ask tab
  downloads `anchorcore-report-<timestamp>.json`.
- **Agents:** `GET /system/report` (`?index=1` for the previous query;
  404 when nothing was asked yet). Documented in the `ask-docs`
  skill — no separate skill; the flow is one call.

The bundle covers the last 20 `/qa` and `/qa/search` calls (in-memory,
metadata only — nothing is written to disk or leaves the machine until
someone shares the file).

## Contents

- `query`: the query text (the reporter's own input — review before
  sharing), kind (`ask`/`search`), scope, timestamp, `retrieval` flag,
  answer `mode`, gate outcome, hit count.
- `hits`: per hit — item pseudonym (`item-1`, stable within the report),
  title hash (sha256, 16 chars, for correlating repeat reports), title
  length, file extension, chunk kind, doc type, env, score, decay,
  boosted flag, `pierced` flag (R47.1 rich-evidence tier pierce), and
  the verbatim leg provenance (`[{leg, rank}]`).
- `index`: source pseudonyms with kind/label/item/error counts, index
  totals, pending embeddings, stale vectors.
- `environment`: app version, `llm_mode`, active embedding family,
  Ollama reachability.

## Redaction contract

The serialized shape contains no document text, summaries, titles, paths,
settings, or secrets **by construction** — the structs lack those fields,
and `tests/api_report.rs` pins the exact key set (any new field fails the
test first). The only free text in the bundle is the query itself.
