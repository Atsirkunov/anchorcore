# Miro connector (R46.5)

Indexes Miro boards as flow text: each board becomes one document whose frames
are sections, whose stickies/shapes/cards/text are bullets in reading order,
and whose connectors are `A → B: caption` lines. No model runs on the way in
(R45.18) — the text is a deterministic rendering of the board's own graph
(`diagram.rs` normal form, shared with draw.io/Excalidraw/Visio).

## Setup

1. In Miro, create an app (Settings → Your apps) with the `boards:read` scope
   and install it on the team; copy its access token.
2. Sources → Add source → **Miro**:
   - **Access token** — stored in the OS keychain (or the encrypted
     `secrets.enc.*` fallback), never in the source config. `GET
     /sources/{id}/config` shows it as `***set***`.
   - **Board IDs** — comma-separated, from the board URL
     (`https://miro.com/app/board/<id>/`), and/or
   - **Team ID** — every board the token can see in that team.

Creating the source does not call Miro (config-only, like Jira); the first sync
reports bad credentials as a source error.

API config shape:

```json
{"connector": "miro", "name": "Whiteboards",
 "config": {"token": "<access token>", "boards": "uXjVO1abc=,uXjVO2def=", "team_id": ""}}
```

`base_url` (default `https://api.miro.com`) exists for tests and proxies.

## What gets indexed

| Miro item | Indexed as |
|---|---|
| `frame` | a `##` section titled by the frame |
| `sticky_note`, `shape`, `text` | a bullet (HTML stripped) in its frame, top→bottom then left→right |
| `card`, `app_card` | `title — description` bullet |
| connector | `start → end: caption` line in the start item's frame (dangling ends read `?`) |
| board description | first line of the document |
| `image`, `document`, `embed`, previews | skipped (board images are R46.13) |

Citations point at the board (`source_ref` = board URL); a frame opens directly
with `https://miro.com/app/board/<id>/?moveToWidget=<frame id>`.

## Sync behavior

- **Incremental per board:** the board's `modifiedAt` is stamped after each
  sync (`sources.last_sync_cursor`). A board whose `modifiedAt` is unchanged —
  and whose stored item finished processing — is not fetched again. A changed
  board is refetched; if its rendered text is identical it still hash-skips.
- **Deletions:** a board that disappears (deleted, or removed from the list,
  or gone from the team) has its item purged. A failed fetch never purges, and
  an empty result purges nothing.
- **Rate limits:** `429` responses retry up to 4 times, waiting `Retry-After`
  (else `X-RateLimit-Reset`, else exponential backoff), at most 60 s per wait.
