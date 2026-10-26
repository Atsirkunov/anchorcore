#!/usr/bin/env bash
# R20.10: end-to-end MCP search smoke (CI, and locally the same way).
#
#   scripts/ci_mcp_search_smoke.sh <anchorcore binary> [repo root]
#
# Starts the app on a free port with a fresh data dir, creates a folder source
# over the repo `sample/` corpus through REST, syncs it, then drives
# `anchorcore mcp` over stdio (ANCHOR_BACKEND_URL) and requires a `search`
# tools/call to return the billing material. SMOKE_TOOL overrides the tool name
# (a wrong name must fail — that is how the smoke itself is proven).
#
# Every HTTP call is `curl -sS -o FILE -w %{http_code}` and every field is read
# from that FILE by `field` — no pipelines, so a failure shows its real body.
set -euo pipefail

BIN=${1:?usage: ci_mcp_search_smoke.sh <anchorcore binary> [repo root]}
ROOT=${2:-$(pwd)}
TOOL=${SMOKE_TOOL:-search}
PORT=${SMOKE_PORT:-18931}
BASE="http://127.0.0.1:$PORT"
WORK=$(mktemp -d)
SERVER_PID=""

cleanup() {
  if [ -n "$SERVER_PID" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() {
  echo "FAULT: $*"
  if [ -f "$WORK/server.log" ]; then echo "--- server log (tail) ---"; tail -n 40 "$WORK/server.log"; fi
  exit 1
}

# field FILE KEY — print a top-level JSON field of FILE (empty when absent).
field() {
  python3 -c 'import json,sys
v = json.load(open(sys.argv[1])).get(sys.argv[2], "")
print(v if not isinstance(v, (dict, list)) else json.dumps(v))' "$1" "$2"
}

# http METHOD PATH OUTFILE [JSON BODY] — prints the status code.
http() {
  local method=$1 path=$2 out=$3 body=${4:-}
  if [ -n "$body" ]; then
    curl -sS -o "$out" -w '%{http_code}' -X "$method" "$BASE$path" \
      -H "X-CSRF-Token: ${CSRF:-}" -H 'content-type: application/json' -d "$body"
  else
    curl -sS -o "$out" -w '%{http_code}' -X "$method" "$BASE$path" -H "X-CSRF-Token: ${CSRF:-}"
  fi
}

[ -x "$BIN" ] || fail "binary not executable: $BIN"
[ -d "$ROOT/sample" ] || fail "no sample/ corpus under $ROOT"

ANCHOR_SECRETS_NO_KEYRING=1 ANCHOR_OPEN_BROWSER=0 \
  ANCHOR_OLLAMA_BASE_URL=http://127.0.0.1:1 ANCHOR_EMBED_BASE_URL=http://127.0.0.1:1 \
  ANCHOR_EMBED_PROVIDER=ollama ANCHOR_HTTP_CONNECT_TIMEOUT=0.2 \
  "$BIN" --port "$PORT" --data-dir "$WORK/data" --host 127.0.0.1 --no-tray >"$WORK/server.log" 2>&1 &
SERVER_PID=$!

code=000
for _ in $(seq 1 60); do
  code=$(curl -sS -o "$WORK/health.json" -w '%{http_code}' "$BASE/health" 2>/dev/null || true)
  [ "$code" = 200 ] && break
  kill -0 "$SERVER_PID" 2>/dev/null || fail "server exited during startup"
  sleep 0.5
done
[ "$code" = 200 ] || fail "server never healthy (last /health status $code)"

code=$(http GET /csrf "$WORK/csrf.json")
[ "$code" = 200 ] || fail "GET /csrf -> $code: $(cat "$WORK/csrf.json")"
CSRF=$(field "$WORK/csrf.json" csrf_token)
[ -n "$CSRF" ] || fail "no csrf_token in $(cat "$WORK/csrf.json")"

code=$(http POST /sources "$WORK/source.json" \
  "{\"connector\":\"folder\",\"name\":\"sample\",\"config\":{\"path\":\"$ROOT/sample\"}}")
[ "$code" = 201 ] || fail "POST /sources -> $code: $(cat "$WORK/source.json")"
SID=$(field "$WORK/source.json" id)

code=$(http POST "/sources/$SID/sync" "$WORK/job.json")
[ "$code" = 202 ] || fail "POST /sources/$SID/sync -> $code: $(cat "$WORK/job.json")"
JID=$(field "$WORK/job.json" id)

status=pending
for _ in $(seq 1 120); do
  code=$(http GET "/sources/jobs/$JID" "$WORK/job.json")
  [ "$code" = 200 ] || fail "GET /sources/jobs/$JID -> $code: $(cat "$WORK/job.json")"
  status=$(field "$WORK/job.json" status)
  [ "$status" = running ] || [ "$status" = pending ] || break
  sleep 0.5
done
[ "$status" = done ] || fail "sync job $JID ended '$status': $(cat "$WORK/job.json")"
echo "ok: sample/ synced through REST (source $SID, job $JID)"

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"ci-smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$TOOL\",\"arguments\":{\"query\":\"billing provider migration\",\"k\":5}}}" \
  >"$WORK/mcp-in.jsonl"
mcp_rc=0
ANCHOR_BACKEND_URL="$BASE" "$BIN" mcp <"$WORK/mcp-in.jsonl" >"$WORK/mcp-out.jsonl" 2>"$WORK/mcp-err.log" || mcp_rc=$?
echo "mcp exit $mcp_rc; stdout (first 1500 bytes):"
head -c 1500 "$WORK/mcp-out.jsonl"; echo

# The tools/call reply (id 2) must be a non-error result whose hits include the
# sample corpus billing documents (by file name).
python3 - "$WORK/mcp-out.jsonl" <<'PY' || fail "MCP search did not return the billing material (stderr: $(cat "$WORK/mcp-err.log"))"
import json, sys
replies = {}
for line in open(sys.argv[1]):
    line = line.strip()
    if line:
        msg = json.loads(line)
        replies[msg.get("id")] = msg
reply = replies.get(2)
if reply is None:
    sys.exit("no reply to tools/call (id 2)")
result = reply.get("result") or {}
if reply.get("error") or result.get("isError"):
    sys.exit("tools/call failed: " + json.dumps(reply)[:500])
text = "".join(c.get("text", "") for c in result.get("content", []))
hits = json.loads(text).get("hits", [])
titles = [str(h.get("item_title", "")) for h in hits]
want = {"d2-billing-provider-approval.md", "billing-migration-runbook.md"}
if not want & set(titles):
    sys.exit("no billing document among hits: " + json.dumps(titles))
print("ok: MCP search returned the billing documents:", sorted(want & set(titles)))
PY
echo "ok: end-to-end MCP search smoke passed (REST source -> sync -> stdio search)"
