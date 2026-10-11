# AnchorCore — MCP zero-setup UX (plan)

> Status: accepted 2026-10-11; revised same day — single-executable merge
> (R20.8) and wizard enablement (R20.9) adopted. R20.1–R20.3 + R20.8 + R20.9
> landed (see `rust/BACKLOG.md` Phase 20); R20.4–R20.7 open (runtime
> discovery, release smoke, docs split, stdio-vs-HTTP decision).
> Trigger: a Windows user followed the Agent-access card in the shipped UI —
> it told them to run `ANCHOR_BACKEND_URL=... cargo run -p anchorcore --bin
> anchorcore-mcp` — which cannot work without a Rust toolchain **and** a C
> linker (MSVC `link.exe` absent; the rustup-bundled MinGW `gcc` is
> link-only, `GCC-WARNING.txt`). Users must never need either.

## What is already true (don't rebuild it)

- Every release zip ships the sidecar: `AnchorCore-windows.zip` =
  `anchorcore.exe` + `anchorcore-mcp.exe`; Linux = `anchorcore` +
  `anchorcore-mcp`; macOS = `AnchorCore.app/Contents/MacOS/anchorcore-mcp`
  (`.github/workflows/release.yml:41-46,74-84,138-145`). Two files today;
  R20.8 merges them into one (see Decisions).
- The sidecar defaults to `http://127.0.0.1:8000` (`bin/mcp.rs:15`) and talks
  HTTP to the running app (single DB owner, like the browser UI). Six
  read-only tools, `public_only` gate, audit rows in `system_events`
  (`component='mcp'`).
- `claude mcp add anchorcore -- <path-to-sidecar>` works today with the
  shipped binary (`docs/mcp.md` §4.1/§8).

## Failure modes found (2026-10-11)

1. **UI lie (fixed, R20.1):** the System tab's "Agent access (MCP)" card
   rendered the dev `cargo run` command as the product's only instruction —
   with Bash env syntax, on Windows.
2. **Doc audience mix (R20.6):** `docs/mcp.md` §4.1/§8 interleaves contributor
   and end-user setup in the same code block; Windows is "`C:\path\to\…`".
3. **No packaging guarantee (R20.5):** nothing fails a release if the sidecar
   is missing or broken; macOS copies it with `if [ -f ]` — a silent skip.
4. **No floor feedback (R20.2, partially fixed):** nothing told the user where
   the sidecar lives on *their* machine, on which port the app listens, or
   whether a call was ever made.

## Target UX — "run once everything works"

Acceptance script (the demo that proves the phase):

1. Clean machine, no Rust, no Node. Download `AnchorCore-windows.zip`
   (or macOS/Linux), unzip, launch the app.
2. System tab shows "Agent access (MCP)" with the **real absolute path** of
   the app and the exact `claude mcp add anchorcore -- "<path>" mcp` line
   (copy button) — zero env vars on the default port. (Pre-R20.8 the path is
   the separate `anchorcore-mcp` file.)
3. Paste once into the harness. Run `search "…"` → cited chunks come back.
4. The card shows "N calls audited, last …" (from `system_events`).
5. Non-default port (`--port 8123`): the card/JSON includes
   `ANCHOR_BACKEND_URL` — or, after R20.4, nothing at all (discovery file).

## Plan (Phase 20 — `rust/BACKLOG.md`)

| # | Task | Priority | ~Est | DoD |
|---|---|---|---|---|
| R20.1 | Kill the lie: UI shows a real command, never `cargo run` | P0 | 0.25d | no `cargo` in `frontend/src`; card renders per-OS command with the resolved path |
| R20.2 | `GET /system/mcp` — sidecar path via `current_exe()` + audit counters | P0 | 0.75d | fresh instance returns correct path on win/linux/mac (incl. inside `.app`) |
| R20.3 | One-click Connect per client (Claude Code, Claude Desktop, Cursor, opencode, Codex): copy + config write with backup/merge, "Repair" when the app moved | P1 | 1.5d | paste-once works with spaces/backslashes/program-files paths |
| R20.4 | Zero-config sidecar: app writes `runtime.json` (port/pid/version); `mcp.rs` reads it when `ANCHOR_BACKEND_URL` is unset | P1 | 0.5d | `--port 8123`, no env vars → tools work |
| R20.5 | CI proves it: hard-assert sidecar in zips; per-OS `mcp-smoke` job drives Inspector CLI (`tools/list`, one `search` on `sample/`) | P1 | 0.5–1d | release fails when the sidecar is missing/broken |
| R20.6 | Audience split: rewrite `docs/mcp.md` setup artifact-first; onboarding wizard final step "Connect your agent"; README quick-start | P1 | 1d | a user path never mentions Rust |
| R20.7 | Strategy: decide local default (stdio + discovery) vs B14.2 streamable HTTP; re-rank B14.2/B14.4 in `docs/mcp.md` §9 | P2 | 0.25d | decision recorded |

| R20.8 | One executable, two modes: `anchorcore` app / `anchorcore mcp` stdio server | P1 | 1d | R20.2 | single `[[bin]]`; zip = one file per OS, one Mach-O in `.app`; `/system/mcp` returns canonical `command` + `args` so the card shows `<path> mcp`; smoke/docs updated |
| R20.9 | Wizard "Connect your agents" step + Settings agent-access card (Connect/Disconnect/Repair) | P1 | 1.5d | R20.3, R20.8 | first-run wizard connects a detected client in one click (`.bak` + merge); Disconnect removes the entry; Repair after the app moves; optional `mcp_enabled` refusal |

Order: R20.1 + R20.2 (shipped 2026-10-11) → **R20.8 first** (one executable
freezes the command shape for everything downstream) → R20.3 + R20.4
(parallel) → R20.9 → R20.5 → R20.6 → R20.7. Total ~7d, parallelizable into
3–4 file-disjoint slices.

## Decisions recorded

- **The sidecar role stays — as a mode, not a second file (R20.8).** It is
  the "single DB owner" design: harnesses spawn `anchorcore mcp` (stdio
  JSON-RPC), which calls the running app over 127.0.0.1. No second writer,
  no lock fights. One executable = one zip entry, one code signature,
  version-locked by construction. This is packaging, not process topology: a
  stdio MCP server must be owned by the harness (stdin/stdout are the
  transport), so runtime stays "one app + one lightweight child per
  harness". Only the B14.2 HTTP transport removes the child.
- **stdio + discovery is the local default; streamable HTTP (B14.2) is the
  Team/remote answer.** HTTP does not replace local stdio; it removes the
  path entirely for shared instances (`claude mcp add --transport http`).
- **Client config writes need explicit user action** (button per client),
  with backup + merge — never clobber an existing `mcpServers` block.
  Read-only display of copy-paste snippets remains the fallback.
- **Rejected: auto-starting the app from the sidecar.** Single DB owner +
  zombie-process confusion; prefer a clear "app not running" error and an
  optional "start at login" toggle later.
- **`/system/mcp` exposes the app's own install path.** Considered
  acceptable (the UI is served by the same app; R10.8 redaction was about
  user data dirs/DB paths); revisit if `/system/*` ever becomes a hosted
  public surface.

## Contributor note (why `cargo run` failed on this box)

`cargo run -p anchorcore --bin anchorcore-mcp` builds `ring` + bundled
SQLite, so it needs a C toolchain. Windows without Visual Studio Build Tools:
install `BrechtSanders.WinLibs.POSIX.UCRT` (winget) and build with
`cargo +stable-x86_64-pc-windows-gnu` (the rustup GNU toolchain alone is not
enough — its bundled `gcc` refuses C files, see `GCC-WARNING.txt`). This is
contributor-only; shipped builds come from CI.

---

*Companion docs: [mcp.md](./mcp.md), [packaging.md](./packaging.md),
[product-plan.md](./product-plan.md), `rust/BACKLOG.md` Phase 20.*
