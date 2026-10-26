# AnchorCore — Releasing

> Every release ships fresh executables for all platforms. Never hand a
> tester an executable built from an older commit — the packaged app embeds
> the frontend at build time, so UI changes silently don't exist in stale
> builds.

## The rule

**Tag = rebuild everything.** Pushing a `v*` tag triggers
`.github/workflows/release.yml`, which:

1. Builds the frontend (`npm run build`)
2. Builds Rust `AnchorCore-{windows,macos,linux}.zip` (`cargo build --release`, `frontend/dist` embedded via `include_dir!`, `codesign` on macOS) plus `AnchorCore-macos.dmg` (R41.2: the same ad-hoc-signed `AnchorCore.app` + an `/Applications` symlink, UDZO via `scripts/make_macos_dmg.sh`; `scripts/verify_macos_dmg.sh` mounts it, checks the layout + `codesign --verify --deep --strict`, runs the MCP handshake against the mounted binary, detaches)
3. Attaches the DMG, the three zips and `SHA256SUMS` to the GitHub Release for that tag (website Download buttons link straight at `AnchorCore-macos.dmg` and the windows/linux zips — keep those file names stable). `AnchorCore-macos.zip` keeps shipping for now: the in-app update card (`system.rs` `update_asset_name`), the setup skill's unzip step and older links still point at it; drop it only after those move to the DMG
4. Deploys `website/` to `anchorcore.dev` via wrangler — but only when `website/` changed since the previous tag (`deploy-website` job diffs `$PREV_TAG..$TAG`). Uses repo secrets `CLOUDFLARE_API_TOKEN` + `CLOUDFLARE_ACCOUNT_ID` (set 2026-10-11; token = Account → Workers Scripts: Edit + Zone `anchorcore.dev` → Workers Routes: Edit — no Client IP filtering, GitHub runners have no stable IPs). If the job fails: rotate the token, re-set the secret, then `gh run rerun <run-id> --failed --repo Atsirkunov/anchorcore` — no re-tag needed

## Docs-only pushes skip CI

`ci.yml` ignores pushes whose changed files are all docs: root `*.md`, `docs/**`, `rust/*.md` (backlog, agent guides), `rust/docs/**`. A push that touches anything else, even one code file, runs the full suite. Tag pushes ignore path filters, so a `v*` tag always runs full CI + `release.yml` — release-notes commits are covered there. For a one-off docs change outside those paths, put `[skip ci]` in the commit message. Markdown that the build or tests read is deliberately **not** ignored: `.agents/skills/*/SKILL.md` (compiled in), `sample/`, `sample-ai/`, `tests/eval_data/` (fixtures). `docs/guides/` feeds the website build, so a broken guide only fails CI on the next tag.

## Release checklist (Rust — shipped)

1. **Verify locally:**
   - [ ] `cargo test -p anchorcore` + `cargo check` clean — Rust green (unit + API contract + golden eval are all inside `cargo test`; no separate conformance step since R45.16)
   - [ ] `npm run build` + `npm run lint` + `npm run test` in `frontend/` — TypeScript + Vite clean
   - [ ] Sanity: `cargo run -p anchorcore -- --port 8000` → ask a question, check Settings
2. **Bump the version** — edit `rust/Cargo.toml:6` `workspace.package.version` → `python scripts/sync_version.py --check` (Cargo is the only source since the archive)
3. **Update docs** — backlog items marked `DONE`; handover `docs/archive/handover-*.md`; stale-claim sweep: `git grep -n "shipped 1\.0\.\|shipped as of\|shipped (1\.0" AGENTS.md README.md docs/packaging.md docs/architecture.md rust/README.md docs/releasing.md` (fix current-version claims, confirm historical ones) + `git grep -n "bundled sample"` (B50) — fix or confirm historical
4. **Draft the release notes** — `docs/release-notes/vX.Y.Z.md` from the commits since the previous tag + the shipped log + handovers, written for users (and their agents), not maintainers: `Highlights` (what's new, plain verbs), `Fixed` (user-visible bugs only), `Upgrade notes` (migrations, behavior changes, anything to re-run). Translate R-codes/B-codes into plain language, never paste internal IDs unexplained. Commit the file with the release. (Notes exist from v1.0.22 onward — do not backfill older tags from memory.)
5. **Tag and push:**
   ```
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
6. **Verify the Release** (github.com → Releases): `AnchorCore-macos.dmg`, `AnchorCore-{windows,macos,linux}.zip` and `SHA256SUMS` present; download on clean machines (macOS: open the DMG, drag to Applications, launch from `/Applications`; others: unzip), run, smoke `curl /health`
7. **Attach the notes** — after CI publishes the release: `gh release edit vX.Y.Z --notes-file docs/release-notes/vX.Y.Z.md` (no empty Release body — a release without notes is incomplete)
8. **Hand off** — testers get the GitHub Release URL, not a repo checkout

## Why rebuild is mandatory

Rust binary embeds `frontend/dist` via `include_dir!` at compile time (`rust/crates/anchorcore/src/frontend.rs:1`). A stale binary shows stale UI even though source has it (classic B23 symptom: "API key field missing"). `release.yml` makes rebuilds automatic.

## Manual rebuild (fallback, dev machine)

```bash
cargo build --release
# artifact: rust/target/release/anchorcore (9.8M, app + `anchorcore mcp` mode)
# full zips, same layout as CI:
.\build.ps1          # Windows: dist/AnchorCore-windows.zip (Rust)
./build.sh           # macOS only: dist/AnchorCore-macos.dmg + .zip (.app, ad-hoc signed; runs the same DMG checks as CI)
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| UI missing a feature that's in source | stale exe (frontend bundled at build time) | rebuild / re-tag |
| macOS "cannot be verified" | no notarization (free tier) | System Settings → Privacy & Security → Open Anyway (right-click → Open no longer works on recent macOS) |
| Release has no artifacts | workflow failed | check Actions tab logs; fix, re-tag |

---

*Companion docs: [packaging.md](./packaging.md), [product-plan.md](./product-plan.md)*
