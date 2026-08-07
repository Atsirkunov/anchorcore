# AnchorCore — Releasing

> Every release ships fresh executables for all platforms. Never hand a
> tester an executable built from an older commit — the packaged app embeds
> the frontend at build time, so UI changes silently don't exist in stale
> builds.

## The rule

**Tag = rebuild everything.** Pushing a `v*` tag triggers
`.github/workflows/release.yml`, which:

1. Builds the frontend (`npm run build`)
2. Builds the Windows exe (PyInstaller, Windows runner)
3. Builds the macOS app (PyInstaller, macOS runner) + ad-hoc signs it
4. Attaches both to the GitHub Release for that tag

## Release checklist

1. **Verify locally** (Windows, the dev machine):
   - [ ] `python -m pytest tests -q` from `backend/` — full suite green
   - [ ] `npm run build` in `frontend/` — TypeScript + Vite clean
   - [ ] Sanity: boot the app, ask a question, check the Settings tab
2. **Bump the version** (in `backend/app/routers/system.py` `APP_VERSION`
   and `backend/app/main.py` FastAPI `version=`)
3. **Update docs** — backlog items marked done; changelog if we keep one
4. **Tag and push**:
   ```
   git tag v0.x.0
   git push origin v0.x.0
   ```
5. **Verify the Release** (github.com → Releases): both artifacts present;
   download the Windows exe and smoke-test it on a clean machine
6. **Hand off** — testers get the GitHub Release URL, not a repo checkout

## Why rebuild is mandatory

`packaging.spec` bundles `frontend/dist` into the exe. The backend serves
the bundled copy, so a stale exe shows stale UI even though the backend code
on disk is current (classic B23 symptom: "API key field missing" while the
source had it). The release workflow makes rebuilds automatic instead of
remembered.

## Manual rebuild (fallback, dev machine)

```powershell
.\build.ps1          # Windows: dist/AnchorCore.exe
./build.sh           # macOS: dist/AnchorCore (ad-hoc signed)
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| UI missing a feature that's in source | stale exe (frontend bundled at build time) | rebuild / re-tag |
| macOS "cannot be verified" | no notarization (free tier) | right-click → Open, or Open Anyway (B24) |
| Release has no artifacts | workflow failed | check Actions tab logs; fix, re-tag |

---

*Companion docs: [packaging.md](./packaging.md), [product-plan.md](./product-plan.md)*
