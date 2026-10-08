#!/usr/bin/env bash
# AnchorCore packaged build (B24 + Rust shipped 1.0.10). Run from repo root:
#   ./build.sh
# Produces dist/AnchorCore.app — double-click macOS app (no terminal, opens browser),
# ad-hoc signed so Gatekeeper shows "Open" on first run. Rust binary is the shipped
# artifact (9.9M, frontend/dist embedded via include_dir!); Python PyInstaller kept
# as legacy `dist/AnchorCore-py.app` for hosted comparison.
# Requires: Rust 1.97.1 ($HOME/.cargo/env), Node 20.
set -euo pipefail

cd "$(dirname "$0")"
ROOT="$(pwd)"
VERSION="$(grep -E '^version = ' "$ROOT/rust/Cargo.toml" | head -1 | sed -E 's/.*\"([^\"]+)\".*/\1/')"

# 0. Kill a running app so the binary isn't locked while we overwrite it.
pkill -f "dist/AnchorCore.app" 2>/dev/null || true
pkill -f "dist/AnchorCore" 2>/dev/null || true
sleep 0.5

# 1. Frontend build (embedded into Rust binary via include_dir!)
echo "==> Building frontend (npm run build)..."
(cd "$ROOT/frontend" && npm run build)

# 2. Rust release build (statically-linked, lto+strip, 9.9M)
echo "==> Building Rust anchorcore $VERSION (cargo build --release)..."
source "$HOME/.cargo/env" 2>/dev/null || true
(cd "$ROOT/rust" && cargo build --release)

# 3. Create double-click .app bundle wrapping the Rust binary
echo "==> Creating dist/AnchorCore.app (Rust)..."
rm -rf "$ROOT/dist/AnchorCore.app"
mkdir -p "$ROOT/dist/AnchorCore.app/Contents/MacOS"
mkdir -p "$ROOT/dist/AnchorCore.app/Contents/Resources"
cp "$ROOT/rust/target/release/anchorcore" "$ROOT/dist/AnchorCore.app/Contents/MacOS/AnchorCore"
chmod +x "$ROOT/dist/AnchorCore.app/Contents/MacOS/AnchorCore"
# Also bundle the MCP sidecar next to it (optional, for completeness)
if [ -f "$ROOT/rust/target/release/anchorcore-mcp" ]; then
  cp "$ROOT/rust/target/release/anchorcore-mcp" "$ROOT/dist/AnchorCore.app/Contents/MacOS/anchorcore-mcp" 2>/dev/null || true
fi
cat > "$ROOT/dist/AnchorCore.app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>AnchorCore</string>
  <key>CFBundleIdentifier</key><string>com.anchorcore.app</string>
  <key>CFBundleName</key><string>AnchorCore</string>
  <key>CFBundleDisplayName</key><string>AnchorCore</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>10.13</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSUIElement</key><false/>
</dict>
</plist>

# 4. Ad-hoc sign the .app so Gatekeeper shows "Open" instead of blocking outright
echo "==> Ad-hoc signing dist/AnchorCore.app (Rust $VERSION)..."
codesign --force --deep --sign - "$ROOT/dist/AnchorCore.app"
codesign --verify --verbose=2 "$ROOT/dist/AnchorCore.app" 2>&1 | head -n 5 || true

# 5. Zip for distribution (Finder shows .app as one file; recipients unzip once)
# Same name as the CI asset (release.yml build-rust-macos).
echo "==> Zipping dist/AnchorCore.app..."
rm -f "$ROOT/dist/AnchorCore-macos.zip"
(cd "$ROOT/dist" && zip -rq AnchorCore-macos.zip AnchorCore.app)

echo ""
echo "Done: $ROOT/dist/AnchorCore-macos.zip (Rust $VERSION, double-click, opens http://127.0.0.1:8123)"
echo "Unzip it, then double-click AnchorCore.app. First run creates"
echo "~/.anchorcore data dir and opens http://127.0.0.1:8123 (set ANCHOR_OPEN_BROWSER=0 to disable)"
