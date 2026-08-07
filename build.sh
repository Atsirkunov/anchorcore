#!/usr/bin/env bash
# AnchorCore packaged build (B24). Run from repo root:
#   ./build.sh
# Produces dist/AnchorCore.app — a windowed macOS app (no terminal window),
# ad-hoc signed so Gatekeeper shows a friendly "Open" prompt on first run.
# Requires: Python 3.12 venv deps installed (backend/.venv), Node deps installed.
set -euo pipefail

cd "$(dirname "$0")"
ROOT="$(pwd)"

# 0. Kill a running app so the binary isn't locked while we overwrite it.
pkill -f "dist/AnchorCore.app" 2>/dev/null || true
pkill -f "dist/AnchorCore" 2>/dev/null || true
sleep 0.5

# 1. Frontend build (bundled into the binary)
echo "==> Building frontend (npm run build)..."
(cd "$ROOT/frontend" && npm run build)

# 2. Backend deps (create the venv if missing)
VENV_PY="$ROOT/backend/.venv/bin/python"
if [ ! -x "$VENV_PY" ]; then
    echo "==> Creating backend/.venv ..."
    python3 -m venv "$ROOT/backend/.venv"
    "$VENV_PY" -m pip install --upgrade pip
    "$VENV_PY" -m pip install -r "$ROOT/backend/requirements.txt"
fi
if ! "$VENV_PY" -c "import PyInstaller" 2>/dev/null; then
    echo "==> Installing build deps (pyinstaller)..."
    "$VENV_PY" -m pip install pyinstaller
fi

# 3. PyInstaller (onedir → dist/AnchorCore.app, windowed — no Terminal window)
echo "==> Building AnchorCore (this takes a minute)..."
(cd "$ROOT" && "$VENV_PY" -m PyInstaller --noconfirm packaging.spec)

# 4. Ad-hoc sign the .app so macOS Gatekeeper shows "Open" instead of blocking outright
echo "==> Ad-hoc signing dist/AnchorCore.app..."
codesign --force --deep --sign - "$ROOT/dist/AnchorCore.app"

# 5. Zip for distribution (Finder shows .app as one file; recipients unzip once)
echo "==> Zipping dist/AnchorCore.app..."
rm -f "$ROOT/dist/AnchorCore-macos.zip"
(cd "$ROOT/dist" && zip -rq AnchorCore-macos.zip AnchorCore.app)

echo ""
echo "Done: $ROOT/dist/AnchorCore-macos.zip"
echo "Unzip it, then double-click AnchorCore.app. First run creates"
echo "~/.anchorcore data dir, starts Ollama if present, opens http://127.0.0.1:8000"
