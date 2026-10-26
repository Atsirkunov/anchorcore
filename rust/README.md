# AnchorCore — Rust workspace

The shipped app: one Axum + rusqlite binary (`crates/anchorcore`) that serves the REST API, the embedded
React UI and the MCP stdio mode (`anchorcore mcp`). Version lives in `Cargo.toml`
(`workspace.package.version`).

```bash
cargo run -p anchorcore -- --port 8000 --data-dir ~/.anchorcore   # dev server
cargo test -p anchorcore                                          # unit + API contract + golden eval
RUSTFLAGS="-D warnings" cargo check -p anchorcore                 # strict check
cargo build -p anchorcore --release                               # embeds ../frontend/dist — build it first
```

- Guide for contributors and coding agents: root [`AGENTS.md`](../AGENTS.md), plus [`AGENTS.md`](./AGENTS.md) here
  for the complexity gate.
- Open work: [`BACKLOG.md`](./BACKLOG.md) · decisions: [`docs/decisions.md`](./docs/decisions.md).
- Releasing: [`docs/releasing.md`](../docs/releasing.md).
