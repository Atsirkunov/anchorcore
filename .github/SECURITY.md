# Security policy

## Reporting a vulnerability

Please do **not** open a public issue for security problems. Use GitHub's
[private vulnerability reporting](https://github.com/Atsirkunov/anchorcore/security/advisories/new)
or email **alex@anchorcore.dev**. We aim to acknowledge within 3 working days.

Please include: what you found, how to reproduce it, the release you tested
(the `AnchorCore-*.zip` name or `vX.Y.Z`) and your platform.

## Supported versions

Only the latest release is supported; older builds are not patched.

## Scope

In scope:

- The desktop app — local HTTP API, ingestion pipeline, secret storage,
  retrieval and citation path.
- The self-hosted team container (`hosting/`, one Rust image) — authentication,
  license gate (`ANCHOR_EDITION=team`, `rust/crates/anchorcore/src/license.rs`),
  multi-user data separation.
- The MCP sidecar (`anchorcore-mcp`) — `public_only` gates and audit logging.

Out of scope:

- Anything that requires an attacker to already control the machine or the data
  directory (`~/.anchorcore`).
- This website (`anchorcore.dev`) — static, no user data beyond the waitlist
  form; report those by email anyway.
- Automated-scanner findings without demonstrated impact.

## What the app is designed to do

Useful when judging whether something is a bug:

- It listens on `127.0.0.1` only, and rejects cross-origin reads/writes and
  requests without the per-session CSRF token (`rust/crates/anchorcore/src/security.rs`).
- Source credentials live in the OS keychain (or an encrypted-file fallback),
  never in the database, and are never returned to the UI (`***set***`).
- `sensitive` / `pii` sources and PII-flagged chunks are sent to cloud models
  only when that provider is explicitly trusted (`ANCHOR_TRUSTED_PROVIDERS`,
  or the broader `ANCHOR_CLOUD_TRUST=1`); refusals are recorded as events.
- There is no telemetry: outbound requests go to the model, Drive, Jira, Linear
  or REST endpoints you configure, and nowhere else.
- The team edition enforces its license gate at startup (`license.rs`);
  circumventing it is a license violation (see `LICENSE.md`), not a security bug —
  report it as a license issue instead.
