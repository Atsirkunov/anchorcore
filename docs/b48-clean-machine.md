# B48 — Clean-machine end-to-end script (v1.0.13)

> B48 DoD: both platforms pass this script; every papercut filed as a backlog item.
> Scope: download → unzip → run → sync `sample/` → cited answers, on clean
> Windows + macOS; MCP smoke via one harness; fresh-clone `docker build` + Team
> image boots with a valid license and refuses without one.

Honest labels used in this doc:

- **Simulated-clean** — isolated dir + fresh data dir + no repo on PATH, but dev
  tools exist on the box. Catches first-run-path bugs, not "missing runtime" bugs.
- **True-clean** — Sandbox / spare machine / fresh Mac with no dev tools. Only
  this catches missing-VC++-runtime / firewall-prompt class bugs.

## 0. Assets under test

Version: `v1.0.13` (`rust/Cargo.toml` `workspace.package.version`).

| Asset | Contents | URL |
|---|---|---|
| `AnchorCore-windows.zip` | `anchorcore.exe` + `anchorcore-mcp.exe` | `https://github.com/Atsirkunov/anchorcore/releases/download/v1.0.13/AnchorCore-windows.zip` |
| `AnchorCore-macos.zip` | `AnchorCore.app` (ad-hoc signed) | `https://github.com/Atsirkunov/anchorcore/releases/download/v1.0.13/AnchorCore-macos.zip` |
| `AnchorCore-linux.zip` | `anchorcore` + `anchorcore-mcp` | `https://github.com/Atsirkunov/anchorcore/releases/download/v1.0.13/AnchorCore-linux.zip` |

Alternate entry: `https://anchorcore.dev` Download buttons (must resolve to the
same assets — verify sizes match the Release page).

**Known gap, read first:** the zips contain **no `sample/` corpus** (binaries
only). Every leg below fetches `sample/` separately from the repo. If that
annoys you, that's a papercut — file it, don't silently work around it.

Sample corpus = 10 files (`sample/people.md`, `sample/sprint12.md`,
`sample/decisions/d1..d4`, `sample/meetings/m1..m2`, `sample/specs/*.md`).

## 1. Common checklist (both platforms)

Run top to bottom. Expected output after each step; any deviation = papercut
(backlog item), not a silent pass.

### 1.1 Download

Download the platform zip fresh (no reuse of an older build — the exe embeds
`frontend/dist` at compile time, so stale builds show stale UI).

Expected: zip present, size matches the GitHub Release page for `v1.0.13`.

### 1.2 Unzip to an isolated dir

Windows: unzip to e.g. `C:\Temp\b48\app\` (no repo on PATH, no dev checkout
involved). macOS: unzip to e.g. `~/b48/app/`, drag `AnchorCore.app` out if you
like — the script below runs the bundle binary directly.

Expected: `anchorcore.exe` + `anchorcore-mcp.exe` (Windows) or
`AnchorCore.app/Contents/MacOS/AnchorCore` + `anchorcore-mcp` (macOS).

### 1.3 Fetch the sample corpus (separate — not in the zip)

```bash
# any machine with git (or download the folder from GitHub in a browser):
git clone --depth 1 --filter=blob:none --sparse https://github.com/Atsirkunov/anchorcore.git b48-sample
git -C b48-sample sparse-checkout set sample
SAMPLE_DIR="$(pwd)/b48-sample/sample"   # Windows PowerShell: $SAMPLE_DIR = "$PWD\b48-sample\sample"
```

Expected: `$SAMPLE_DIR` holds 10 files (`people.md`, `sprint12.md`, …).

### 1.4 First run (fresh data dir, no browser)

Windows (PowerShell, from the isolated dir):

```powershell
$env:ANCHOR_OPEN_BROWSER = "0"
.\anchorcore.exe --port 8123 --data-dir C:\Temp\b48\data
```

macOS (Terminal):

```bash
ANCHOR_OPEN_BROWSER=0 ./AnchorCore.app/Contents/MacOS/AnchorCore --port 8123 --data-dir ~/b48/data
```

(Use port `8123`, not `8000`, so a dev instance on `:8000` can't mask a
startup failure. First-timers double-clicking get `:8000` + browser — same
binary, same migrations.)

Expected:

- Process stays up, log line `listening on 127.0.0.1:8123`.
- Data dir created with `anchorcore.db` + `anchorcore.log`.
- No firewall/V C++-runtime popup on true-clean (if one appears, that's a finding).

### 1.5 `GET /health` → 200 `status: ok`

```bash
curl -sf http://127.0.0.1:8123/health
```

Expected (shape, values vary):

```json
{"status":"ok","data_dir":"redacted","components":{"ollama":"ok|offline","answer_key":"...","pending_embeddings":0,...}}
```

`ollama: offline` on a box without Ollama is **not** a failure — the suite is
designed to pass degraded (rule-based classifier, keyword retrieval).

All mutating calls below (`POST`/`PUT`/`PATCH`/`DELETE`) require an
`X-CSRF-Token` header — without it they `403` (found on the Windows leg,
2026-10-09). Grab one now and reuse it for the whole session:

```bash
CSRF=$(curl -sf http://127.0.0.1:8123/csrf | python3 -c 'import json,sys; print(json.load(sys.stdin)["csrf_token"])')
# Windows PowerShell: $CSRF = (Invoke-RestMethod http://127.0.0.1:8123/csrf).csrf_token
```

### 1.6 Create a folder source pointing at `sample/`

```bash
curl -sf -X POST http://127.0.0.1:8123/sources \
  -H 'Content-Type: application/json' \
  -H "X-CSRF-Token: $CSRF" \
  -d "{\"name\":\"Sample company\",\"connector\":\"folder\",\"config\":{\"path\":\"$SAMPLE_DIR\"}}"
# Windows PowerShell: replace $SAMPLE_DIR with $env:SAMPLE_DIR, $CSRF with $env:CSRF, and mind quoting
```

Expected: `201` with a JSON body containing `"id": <n>`. Record `<n>` as `SOURCE_ID`.

### 1.7 Sync → job `done`

```bash
curl -sf -X POST http://127.0.0.1:8123/sources/$SOURCE_ID/sync -H "X-CSRF-Token: $CSRF"
# → {"id": <job>, ...}; then poll:
curl -sf http://127.0.0.1:8123/sources/jobs/$JOB_ID
```

Expected: `status` reaches `"done"` (`pending` → `running` → `done`).
10 files, no Ollama: well under 60s. `failed` (with `error`) = stop, file it.

### 1.8 Ask #1 — decision + reasoning, cited

```bash
curl -sf -X POST http://127.0.0.1:8123/qa \
  -H 'Content-Type: application/json' \
  -H "X-CSRF-Token: $CSRF" \
  -d '{"question":"What was decided about security transfers, and why?"}'
```

Expected:

- `answer`: non-empty. Without Ollama/API key it's extractive, not prose —
  **pass criterion is citations, not eloquence**.
- `citations`: array with **≥1** entry; each has `source_ref` + `path`
  (e.g. a `d1-security-transfers-scope` ref or an `Art › 12.3`-style section
  path / chunk link). Empty citations = FAIL, file it.

### 1.9 Ask #2 — ownership, cited

```bash
curl -sf -X POST http://127.0.0.1:8123/qa \
  -H 'Content-Type: application/json' \
  -H "X-CSRF-Token: $CSRF" \
  -d '{"question":"Who owns the billing migration?"}'
```

Expected: same shape as 1.8; answer names **Sarah**, cited (decisions/notes).

### 1.10 UI spot-check (human, 30 seconds)

Open `http://127.0.0.1:8123` in a browser: Ask tab shows the two answers with
visible citations; Sources tab shows the synced source. Any white screen /
missing feature that's in source = stale-embed symptom — file it.

### 1.11 Stop + data-dir hygiene

Stop the app (Ctrl-C). Delete the data dir = fresh start (verify a second
launch recreates it cleanly if you have time).

## 2. MCP smoke (one harness — Inspector CLI)

Prereq: the app from §1 still running (the sidecar talks to it over
`ANCHOR_BACKEND_URL`, default `http://127.0.0.1:8000` — override to `:8123`).

```bash
# from the isolated dir (Windows: anchorcore-mcp.exe). NOTE: plain `export`
# does NOT reach the sidecar — Inspector spawns it without your env. Pass the
# backend URL with `-e` (found on the Windows leg, 2026-10-09):
npx -y @modelcontextprotocol/inspector --cli ./anchorcore-mcp \
  -e ANCHOR_BACKEND_URL=http://127.0.0.1:8123 --method tools/list
npx -y @modelcontextprotocol/inspector --cli ./anchorcore-mcp \
  -e ANCHOR_BACKEND_URL=http://127.0.0.1:8123 --method tools/call \
  --tool-name memory_status
# (--tool-arg takes key=value pairs only, and memory_status takes no args, so
# no --tool-arg flag. For a cited ask over MCP:
#  --tool-name ask --tool-arg question="Who owns the billing migration?")
```

Expected:

- `tools/list` returns 6 tools: `ask`, `search`, `get_entity`, `get_source`,
  `list_sources`, `memory_status`.
- `tools/call memory_status` succeeds (version + model availability +
  pending embeddings).

Fallback if `npx` is unavailable (no network/npm): raw JSON-RPC over stdio —
`initialize` → `tools/list` → `tools/call{memory_status}` piped into the
sidecar binary — exercises the same surface. Record which harness ran.

## 3. macOS leg — copy-paste Terminal script (for the Mac owner)

```bash
set -euo pipefail
mkdir -p ~/b48 && cd ~/b48
curl -sL -o macos.zip https://github.com/Atsirkunov/anchorcore/releases/download/v1.0.13/AnchorCore-macos.zip
rm -rf AnchorCore.app && unzip -q macos.zip
# Gatekeeper: ad-hoc signed, no notarization — approve once:
xattr -dr com.apple.quarantine AnchorCore.app || true
git clone --depth 1 --filter=blob:none --sparse https://github.com/Atsirkunov/anchorcore.git b48-sample 2>/dev/null || true
git -C b48-sample sparse-checkout set sample
export SAMPLE_DIR="$PWD/b48-sample/sample" ANCHOR_OPEN_BROWSER=0
ls "$SAMPLE_DIR" | wc -l   # expect 10 entries (files + dirs)
rm -rf ~/b48/data
./AnchorCore.app/Contents/MacOS/AnchorCore --port 8123 --data-dir ~/b48/data > mac-app.log 2>&1 &
APP_PID=$!
trap "kill $APP_PID" EXIT
for i in $(seq 1 20); do curl -sf http://127.0.0.1:8123/health && break; sleep 1; done
curl -sf http://127.0.0.1:8123/health; echo
CSRF=$(curl -sf http://127.0.0.1:8123/csrf | python3 -c 'import json,sys; print(json.load(sys.stdin)["csrf_token"])')
SOURCE_ID=$(curl -sf -X POST http://127.0.0.1:8123/sources -H 'Content-Type: application/json' -H "X-CSRF-Token: $CSRF" \
  -d "{\"name\":\"Sample company\",\"connector\":\"folder\",\"config\":{\"path\":\"$SAMPLE_DIR\"}}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "SOURCE_ID=$SOURCE_ID"
JOB_ID=$(curl -sf -X POST http://127.0.0.1:8123/sources/$SOURCE_ID/sync -H "X-CSRF-Token: $CSRF" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "JOB_ID=$JOB_ID"
for i in $(seq 1 60); do S=$(curl -sf http://127.0.0.1:8123/sources/jobs/$JOB_ID | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])'); echo "job=$S"; [ "$S" = done ] && break; [ "$S" = failed ] && exit 1; sleep 2; done
curl -sf -X POST http://127.0.0.1:8123/qa -H 'Content-Type: application/json' -H "X-CSRF-Token: $CSRF" \
  -d '{"question":"What was decided about security transfers, and why?"}' | python3 -m json.tool
curl -sf -X POST http://127.0.0.1:8123/qa -H 'Content-Type: application/json' -H "X-CSRF-Token: $CSRF" \
  -d '{"question":"Who owns the billing migration?"}' | python3 -m json.tool
kill $APP_PID; trap - EXIT
echo MAC-LEG-DONE
```

Paste back: the full transcript + `mac-app.log` tail on any failure.

## 4. Docker / Team-image leg — copy-paste script (needs Docker Desktop)

```bash
set -euo pipefail
rm -rf /tmp/b48-fresh && git clone https://github.com/Atsirkunov/anchorcore.git /tmp/b48-fresh
cd /tmp/b48-fresh
git checkout main   # or the first release with R19.1 (v1.0.14+)
# --- license roundtrip (documented seller flow) ---
pip install cryptography >/dev/null 2>&1 || true
python scripts/make_license.py gen-keypair   # RECORD the public key
# TEST ONLY: register the throwaway public key in
# rust/crates/anchorcore/src/license.rs (PUBLIC_KEYS_HEX) — do not commit.
export ANCHOR_LICENSE_PRIVATE_KEY=<hex from gen-keypair>
python scripts/make_license.py issue --org "B48 Test" > /tmp/b48-license.json
cat /tmp/b48-license.json
# --- one image: UI + Rust binary, SQLite volume, no Postgres ---
docker build -f hosting/Dockerfile -t anchorcore:team .
# --- refuses WITHOUT a license (Team gate) ---
if docker run --rm --name b48-nolic anchorcore:team; then
  echo "FAIL: booted without a license"; exit 1
else
  echo "refused as expected (non-zero exit)"
fi
# --- boots WITH one ---
docker run -d --name b48-team -p 8123:8000 \
  -v /tmp/b48-license.json:/data/license.json \
  -e ANCHOR_LICENSE_FILE=/data/license.json anchorcore:team
sleep 8; curl -sf http://127.0.0.1:8123/health; echo
curl -s http://127.0.0.1:8123/ | head -c 120; echo   # UI served by the same container
docker logs b48-team 2>&1 | tail -5
docker rm -f b48-team
echo DOCKER-LEG-DONE
```

Expected: build green; licensed boot serves `/health` + the UI from the same
container (no Postgres); unlicensed boot exits 2 with a license error.
**Caveat:** the image verifies against `PUBLIC_KEYS_HEX` in
`rust/crates/anchorcore/src/license.rs` (production key) — a freshly minted
keypair verifies ONLY after registering it there and rebuilding. If refusal
happens even with the fresh license, paste the log line: that decides whether
it's a test-setup artifact or a real bug.

## 5. Recording results

- Each leg: PASS/FAIL + transcript pasted into the B48 handoff.
- Every deviation → new backlog item (`B50+`) in `docs/product-plan.md`.
- B48 → DONE only when Windows + macOS both pass §1 (either leg may be
  true-clean or simulated-clean, but the label must be recorded honestly).

---

*Companion docs: [product-plan.md](./product-plan.md) (B48), [packaging.md](./packaging.md), [mcp.md](./mcp.md), [sample-dataset.md](./sample-dataset.md)*
