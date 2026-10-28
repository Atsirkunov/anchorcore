# AnchorCore

Ask your files. Get the answer and the exact source.

AnchorCore is a private AI search app. Connect your folders, Drive, Jira, Linear — or any JSON API —
and ask "what was decided about X, and why?" in plain language. You get a cited answer that shows
exactly where it came from. Your index stays on your machine; answers are generated locally unless
you connect a cloud model.

## Download

Free for personal use. No account, no Docker, no setup:

- [Download for Windows](https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-windows.zip)
- [Download for macOS](https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-macos.dmg) (DMG)
- [Download for Linux](https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-linux.zip)

macOS: open the DMG and drag AnchorCore onto Applications. Windows/Linux: unzip. Run it, and the app opens in your browser. Your data stays in `~/.anchorcore`
on your machine. Connectors you enable (Drive, Jira, Linear, REST) and any cloud model
you configure make network requests; see the
[security notes](https://anchorcore.dev/security) for the full list.
The app keeps running in the system tray (Windows) / menu bar (macOS) — quit
it from there; closing the browser tab doesn't stop it. On a Mac with many
menu-bar icons, look left of your other icons and ⌘-drag it where you want it.
Linux has no tray yet.
Verify a download against `SHA256SUMS` on the release page.

> First launch: run AnchorCore from `/Applications`. macOS: open it, then System Settings → Privacy & Security → Open Anyway (the app isn't Apple-notarized yet).
> Windows: SmartScreen → More info → Run anyway. (Windows and Linux builds aren't code-signed; nothing is store-signed.)

## 60-second start

1. Download and run the app (above).
2. Sources → Local folder → point at the `sample/` demo corpus (download
   [sample.zip](https://anchorcore.dev/sample.zip) and unzip it, or use the folder
   in this repo; release zips ship the app only — or use your own files) → Sync.
3. Ask: "What was decided about instant payments, and why?" → cited answer.
4. Optional: **System tab → MCP** connects Claude Code, Codex, opencode, Cursor or
   Claude Desktop to the same index — the app writes the client config for you, so
   there is nothing to build. The onboarding wizard's last step offers the same card.

Tip: for full hybrid search, press **Download built-in model** in the wizard or
Settings (nomic-embed-text v1.5, ~550 MB download, ~275 MB on disk — runs inside
the app, Metal on macOS) — or use [Ollama](https://ollama.com) with
`nomic-embed-text`, which indexes large folders faster and is preferred
whenever it runs; the vectors are identical, so switching costs nothing. Without
either, keyword search still works. Indexing needs no chat model: decisions and
actions are read from record structure by rules.

## Editions

- **Personal** — this app. Free forever for personal use (see [License](#license)).
- **Team (self-hosted)** — one Docker container in your VPC, licensed per quote:
  [request a quote](https://anchorcore.dev/team). See
  [hosting/README.md](./hosting/README.md).
- **Hosted** — coming soon; [join the waitlist](https://anchorcore.dev/hosted).

## License

Free for personal use under [LICENSE.md](./LICENSE.md) (source-available).
Team and commercial use requires a paid license.

## Docs

- [Agent setup skill](./.agents/skills/anchorcore-setup/SKILL.md) — hand it to your agent: install, configure models and sources, connect MCP, verify answers
- [Agent connectivity (MCP)](./docs/mcp.md) — how harnesses (Claude Code, Codex, opencode, Cursor) search your files
- [Security notes](https://anchorcore.dev/security) — network endpoints, label gate, secrets, known limits
- [Backup, export & device migration](./docs/backup.md) — portable `.anchorcore-backup` archives, secrets model, CLI + UI walkthrough
- [Updates](./docs/updates.md) — manual release check (no background pings), the startup-check opt-in, what updating does to your data
- [System tray](./docs/tray.md) — Open + Quit from the tray/menu bar, opt-outs, headless notes
- [REST API guide](./docs/guides/rest-api.md) — connect any JSON list API as a source
- [Obsidian vault guide](./docs/guides/obsidian-vault.md) — point AnchorCore at a vault folder
- [Architecture](./docs/architecture.md) — system view, ingest + Q&A flow, data model
- [Docs index](./docs/README.md) — everything else: plans, packaging, releasing, history, release notes

## Project layout

```
rust/       Rust service (Axum): connectors, ingest, retrieval, MCP — the shipped binary (version: rust/Cargo.toml)
frontend/   React SPA (Vite), embedded into the binary at build time
sample/     Mini-company demo corpus (guide: docs/sample-dataset.md); sample-ai/ is a second one
hosting/    Team image — one container running the same binary (guide: hosting/README.md)
website/    anchorcore.dev
docs/       Architecture, guides, plans, release notes (index: docs/README.md)
```

## Build and develop

- **Run from source:** `cargo run -p anchorcore -- --port 8000 --data-dir ~/.anchorcore` (applies
  migrations, starts the watcher and jobs). Frontend dev: `cd frontend && npm run dev` (:5173, proxies to :8000).
- **Build the app:** `./build.sh` (macOS/Linux) or `.\build.ps1` (Windows). The binary embeds
  `frontend/dist`, so run `npm run build` first after any UI change. Pushing a `v*` tag builds every
  platform zip (`docs/releasing.md`).
- **Tests:** `cargo test -p anchorcore` (unit + black-box API contract tests + golden retrieval eval) and
  `cd frontend && npm run build`. The suite runs without Ollama.
- **Team image:** `docker compose -f hosting/docker-compose.yml up --build` — see [hosting/README.md](./hosting/README.md).
- **Agent skills:** only our own skills in `.agents/skills/` are tracked; vendored ones are restored with
  `npx skills experimental_install` (`skills-lock.json`).
- Contributors and coding agents: start with [AGENTS.md](./AGENTS.md).

The Python backend is archived on tag `archive/python-final` ([docs/archived-python.md](./docs/archived-python.md)).

## Models (Settings tab — no restart)

| What | Local | Cloud |
|---|---|---|
| Embeddings (optional — keyword search works without) | built-in nomic-embed-text v1.5 (one download, no Ollama) or Ollama `nomic-embed-text` | any OpenAI-compatible endpoint + key |
| Answers | **agent mode:** your agent writes them from cited evidence (no model here) · **local mode:** Ollama | local mode: any OpenAI-compatible endpoint + key |

Settings are stored in the app (`app_settings`, overriding env defaults); secrets go to the OS keychain,
never the DB. Env keys are listed in `hosting/.env.example`.

## How it works

Sources are extracted to text (documents, PDFs, Office files, diagrams; images are kept as cited assets),
chunked along their structure, indexed in SQLite FTS5 and — if an embedder is configured — embedded.
Decisions and action items are read from record structure (`Decision:`, `Status:`, `Owner:`, ADR front
matter, action-item checkboxes) by rules; no chat model runs during indexing. Questions fuse keyword and
vector results by reciprocal rank, scoped to a project if you pick one, and come back with section-level
citations — written by your agent (agent mode) or by a local/cloud model (local mode). Sources labelled
`sensitive` never reach an untrusted cloud provider.

Details: [docs/architecture.md](./docs/architecture.md) · retrieval knobs: [docs/guides/retrieval-tuning.md](./docs/guides/retrieval-tuning.md) · agents: [docs/mcp.md](./docs/mcp.md) and the [search skill](./.agents/skills/ask-docs/SKILL.md) (REST).
