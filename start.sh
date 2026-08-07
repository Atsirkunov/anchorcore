#!/usr/bin/env bash
# AnchorCore single run entry point (macOS / Linux).
#   ./start.sh            boots the backend (serves the built UI at http://localhost:8000)
#   ./start.sh --dev      also starts the Vite dev server (http://localhost:5173)
#   ./start.sh --no-ollama   skips auto-starting the local Ollama server
#
# - Kills orphaned processes still holding the port
# - Starts Ollama if it isn't running (skippable)
# - Creates the venv + .env on first run
# - Applies Alembic migrations (stamps legacy DBs, then upgrades)
# - The app prints its effective config banner at boot
set -euo pipefail

cd "$(dirname "$0")"

ROOT="$(pwd)"
BACKEND="$ROOT/backend"
VENV="$BACKEND/.venv"
VENV_PY="$VENV/bin/python"
PORT="${PORT:-8000}"

DEV=""
NO_OLLAMA=""
for arg in "$@"; do
  case "$arg" in
    --dev) DEV="1" ;;
    --no-ollama) NO_OLLAMA="1" ;;
  esac
done

ollama_up() {
  curl -sf --max-time 2 http://127.0.0.1:11434/api/tags >/dev/null 2>&1
}

# 1. Kill orphans on the port (stale processes with old code or locked SQLite).
ORPHANS="$(lsof -ti tcp:"$PORT" 2>/dev/null || true)"
if [ -n "$ORPHANS" ]; then
  echo "Killing orphan(s) on port $PORT: $ORPHANS"
  echo "$ORPHANS" | xargs kill -9 2>/dev/null || true
fi

# 2. Ollama: start the local model server if it isn't already running.
if [ -z "$NO_OLLAMA" ] && ! ollama_up; then
  echo "Ollama is not running - starting it..."
  if command -v ollama >/dev/null 2>&1; then
    nohup ollama serve >/dev/null 2>&1 &
  elif [ -d "/Applications/Ollama.app" ]; then
    open -a Ollama
  else
    echo "WARNING: ollama not found - install it (brew install ollama or https://ollama.com)."
  fi
  for _ in $(seq 1 30); do
    ollama_up && break
    sleep 0.5
  done
fi
if ollama_up; then
  echo "Ollama: up (http://localhost:11434)"
else
  echo "WARNING: Ollama still not reachable - classification will fall back to rules and answers will be limited."
fi

# 3. Environment: venv + .env on first run.
if [ ! -x "$VENV_PY" ]; then
  echo "Creating $VENV ..."
  python3 -m venv "$VENV"
  "$VENV_PY" -m pip install --upgrade pip >/dev/null
  "$VENV_PY" -m pip install -r "$BACKEND/requirements.txt"
fi
if [ ! -f "$BACKEND/.env" ]; then
  cp "$BACKEND/.env.example" "$BACKEND/.env"
  echo "Created backend/.env from .env.example - review it (set ANCHOR_ANSWER_API_KEY or point ANCHOR_ANSWER_BASE_URL at Ollama)."
fi

# 3b. Pull missing models (configured in backend/.env; downloads only what's absent).
if [ -z "$NO_OLLAMA" ] && ollama_up && command -v ollama >/dev/null 2>&1; then
  CONFIGURED="$("$VENV_PY" -c "from app.config import settings; print(settings.classifier_model); print(settings.embed_model)" 2>/dev/null || true)"
  HAVE="$(ollama list 2>/dev/null | tail -n +2 | awk '{print $1}' | sed 's/:.*//')"
  for model in $CONFIGURED; do
    short="${model%%:*}"
    if ! echo "$HAVE" | grep -qx "$short"; then
      echo "Pulling Ollama model $model (first run only)..."
      ollama pull "$model" || echo "WARNING: failed to pull $model - run 'ollama pull $model' manually."
    fi
  done
fi

# 4. Migrations (stamps legacy DBs that predate Alembic, then upgrades).
(cd "$BACKEND" && "$VENV_PY" -c "from app.main import _run_migrations; _run_migrations()")

# 5. Boot.
echo "Starting AnchorCore backend on http://localhost:$PORT (Ctrl+C to stop)..."
if [ -n "$DEV" ]; then
  (cd "$ROOT/frontend" && npm run dev) &
fi
(cd "$BACKEND" && "$VENV_PY" -m uvicorn app.main:app --host 127.0.0.1 --port "$PORT")
