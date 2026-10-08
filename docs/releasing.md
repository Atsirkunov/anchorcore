# AnchorCore — Releasing

> Every release ships fresh executables for all platforms. Never hand a
> tester an executable built from an older commit — the packaged app embeds
> the frontend at build time, so UI changes silently don't exist in stale
> builds.

## The rule

**Tag = rebuild everything.** Pushing a `v*` tag triggers
`.github/workflows/release.yml`, which:

1. Builds the frontend (`npm run build`)
2. Builds Rust `AnchorCore-rust-*` (`cargo build --release`, `frontend/dist` embedded via `include_dir!`, `codesign` on macOS)
3. Attaches to the GitHub Release for that tag (Python `dist/AnchorCore.*` legacy kept for `hosting` reference only)

## Release checklist (Rust — shipped 1.0.12)

1. **Verify locally:**
   - [ ] `cargo test -p anchorcore` (77) + `cargo check 0` — Rust green
   - [ ] `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q` — conformance `116/17`
   - [ ] `npm run build` + `npm run lint` + `npm run test` in `frontend/` — TypeScript + Vite clean
   - [ ] Sanity: `cargo run -p anchorcore -- --port 8000` → ask a question, check Settings
2. **Bump the version** — edit `rust/Cargo.toml:6` `workspace.package.version` → `python scripts/sync_version.py` (writes `backend/app/config.py:11` for `hosting`)
3. **Update docs** — backlog items marked `DONE`; handover `docs/handover-*.md`
4. **Tag and push:**
   ```
   git tag v1.0.12
   git push origin v1.0.12
   ```
5. **Verify the Release** (github.com → Releases): `AnchorCore-rust-*` present; smoke `curl /health` on clean machine
6. **Hand off** — testers get the GitHub Release URL, not a repo checkout

## Why rebuild is mandatory

Rust binary embeds `frontend/dist` via `include_dir!` at compile time (`rust/crates/anchorcore/src/frontend.rs:1`). A stale binary shows stale UI even though source has it (classic B23 symptom: "API key field missing"). `release.yml` makes rebuilds automatic.

## Manual rebuild (fallback, dev machine)

```bash
cargo build --release -p anchorcore
# artifact: rust/target/release/anchorcore (9.8M)
# legacy Python (hosting only):
.\build.ps1          # Windows: dist/AnchorCore.exe (deprecated)
./build.sh           # macOS: dist/AnchorCore (ad-hoc signed, deprecated)
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| UI missing a feature that's in source | stale exe (frontend bundled at build time) | rebuild / re-tag |
| macOS "cannot be verified" | no notarization (free tier) | right-click → Open, or Open Anyway (B24) |
| Release has no artifacts | workflow failed | check Actions tab logs; fix, re-tag |

---

*Companion docs: [packaging.md](./packaging.md), [product-plan.md](./product-plan.md)*
