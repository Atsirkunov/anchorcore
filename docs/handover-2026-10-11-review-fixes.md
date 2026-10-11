# Handover 2026-10-11 — review fixes 1–4 done, item 5 needs an unsandboxed run

For the next agent (unsandboxed Muse). Items 1–4 from the v1.0.16 review are
implemented and verified below; item 5 could not run in the sandbox (no TCP
bind, no network/auth) and is specified step-by-step at the end.

All work is uncommitted in `/Users/aleksandrtsirkunov/anchorcore` on `main`
(`9dfb24d` + working tree). Do not commit unless the owner asks.

## Done and verified (this turn)

1. **Tests for `mcp_setup.rs` + picker components.**
   - `rust/.../src/mcp_setup.rs`: new `#[cfg(test)]` module, 11 tests
     (TOML escape/connect/append/remove, JSON merge+backup, opencode nesting,
     JSONC guard, backup naming, client ids, `finish` wrapper). Touches only
     tempdirs, never real `$HOME`.
   - `frontend/src/FolderPicker.test.tsx` (6 tests) and
     `frontend/src/McpConnect.test.tsx` (5 tests): real render tests via
     `createRoot` + `act` (jsdom is already the vitest environment, no new
     deps), with `api.*` spied. Covers navigation, select/cancel/Escape,
     error surfacing, snippet JSON shapes, connect/disconnect notes.
2. **Rust-side onboarding shape test.** `system.rs`:
   `onboarding_response_shape_is_versioned` pins the exact
   `/system/onboarding` key set (top level + `ollama`/`sample` objects).
   This replaces the frozen `test_onboarding_state` deselected from
   rust-conformance in `ffd3e21`.
3. **`backend/` residue removed.** It held zero `.py` files (only `.venv`,
   `__pycache__`, `.pytest_cache`, `.env`, `data/` with a local db/log/key).
   All 89 sources live on tag `archive/python-final`; CI and
   `rust/scripts/conformance.sh` both restore `backend/` from that tag, so
   conformance self-heals. (Finder fought the delete by recreating
   `.DS_Store` — a retry loop finished it.)
4. **Hardening.**
   - B54 `ollama.rs`: poison-tolerant `lock_pulls()` (no more
     `.lock().unwrap()` in handlers); atomic `claim_from_map` dedupes
     in-flight pulls — response is now
     `{"started": [...], "already_running": [...]}`. Only consumer
     (`useModelPull`) ignores the body and polls, so this is compatible;
     `api.ts` type updated. 3 new claim tests (15/15 `ollama::`).
     Product-plan B54 bullet updated (behavior + count).
   - B59 `main.rs`: `Args::parse()` moved above the license gate so
     `--help`/`--version` work on the Team image without a license.
   - R24.3: `SKIP_DIRS` shadowing caveat documented in `folder.rs`,
     `AGENTS.md`, and the B9 product-plan entry.

Observed green on this box:

- `cargo test -p anchorcore`: **157 passed, 1 failed** — the failure is
  `answer::tests::generate_answer_composed_true_with_mock_server`, which
  dies at `TcpListener::bind` with `PermissionDenied`: sandbox restriction,
  not a code bug (see item 5a).
- `RUSTFLAGS="-D warnings" cargo check -p anchorcore`: clean.
- Frontend: vitest **34/34** (7 files), `eslint src` clean, `tsc -b` clean.
- `scripts/sync_version.py --check`: re-run it if you touch versions
  (was `ok 1.0.16` before these edits; nothing here bumps versions).

## Item 5 — run unsandboxed

### 5a. The sandbox-blocked Rust test

```sh
cd /Users/aleksandrtsirkunov/anchorcore/rust
cargo test -p anchorcore answer::tests::generate_answer_composed_true_with_mock_server
```

Expected: pass (it binds `127.0.0.1:0` for a hermetic mock LLM server).
Then run the full suite and expect **158/158**:

```sh
cargo test -p anchorcore
```

If the single test still fails outside the sandbox, treat it as a real
regression in `answer.rs` `generate_answer` (`composed` flag) and
investigate — do not deselect it.

### 5b. Verify the anchorcore.dev deploy (B47)

Config: `website/wrangler.jsonc` (Static Assets from `./dist`, route
`anchorcore.dev/*`). Cannot verify without network/auth.

1. `cd /Users/aleksandrtsirkunov/anchorcore/website && npm run build`
2. `curl -sI https://anchorcore.dev | head -5` — expect `200`.
3. Fetch the homepage and confirm it matches the local build: it must
   contain the Guides/FAQ entry points, the hero TOC, and reference
   `/og-card.png` (added in `df8722b`). E.g.
   `curl -s https://anchorcore.dev -o /tmp/ac-home.html` and grep for
   those markers; eyeball-diff against `dist/index.html` structure.
4. `curl -sI https://anchorcore.dev/og-card.png` — expect `200`.
5. If wrangler auth is available: `npx wrangler deployments list` and
   confirm the production deployment is newer than tag `v1.0.14`
   (the release that closed B47).

Report back: test counts for 5a, and for 5b the HTTP statuses plus
whether the served homepage matches the local `dist` build (or how it
differs). If the deploy is stale, say so — do not redeploy without the
owner asking.
