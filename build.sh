#!/usr/bin/env bash
# AnchorCore packaged build (B24 + Rust shipped 1.0.20). Run from repo root:
#   ./build.sh
# Produces dist/AnchorCore.app — double-click macOS app (no terminal, opens browser),
# ad-hoc signed (not notarized: first launch needs Open Anyway) — then packs it the
# same way as CI (release.yml build-rust-macos):
#   dist/AnchorCore-macos.dmg  — AnchorCore.app + /Applications symlink (R41.2)
#   dist/AnchorCore-macos.zip  — the same .app, zipped (kept one release)
# Rust binary is the shipped artifact (frontend/dist embedded via include_dir!).
# Requires: macOS (codesign, hdiutil), Rust ($HOME/.cargo/env), Node 20.
set -euo pipefail

[ "$(uname -s)" = "Darwin" ] || { echo "build.sh: macOS only (codesign + hdiutil); use build.ps1 on Windows, cargo build --release on Linux" >&2; exit 1; }

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
# R20.8: `anchorcore mcp` is a mode of the same binary — nothing else to bundle.
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
PLIST

# 4. Sign the .app. R41.4: with NOTARY_PROFILE set (a `notarytool
# store-credentials` profile) and a Developer ID Application identity in the
# keychain, sign like release.yml (hardened runtime + timestamp) and notarize +
# staple; otherwise ad-hoc sign so Gatekeeper shows "Open Anyway".
DEVID=""
if [ -n "${NOTARY_PROFILE:-}" ]; then
  DEVID="$(security find-identity -v -p codesigning | awk '/Developer ID Application/ {print $2; exit}')"
  [ -n "$DEVID" ] || { echo "FAULT: NOTARY_PROFILE is set but no 'Developer ID Application' identity is in the keychain" >&2; exit 1; }
fi
if [ -n "$DEVID" ]; then
  echo "==> Developer ID signing + notarizing dist/AnchorCore.app (Rust $VERSION)..."
  codesign --force --options runtime --timestamp --sign "$DEVID" "$ROOT/dist/AnchorCore.app"
  codesign --verify --deep --strict "$ROOT/dist/AnchorCore.app"
  NOTARY_ZIP="$(mktemp -d "${TMPDIR:-/tmp}/anchorcore-notary.XXXXXX")/AnchorCore.zip"
  ditto -c -k --keepParent "$ROOT/dist/AnchorCore.app" "$NOTARY_ZIP"
  bash "$ROOT/scripts/notarize_macos.sh" "$NOTARY_ZIP" "$ROOT/dist/AnchorCore.app"
  rm -rf "$(dirname "$NOTARY_ZIP")"
else
  echo "==> Ad-hoc signing dist/AnchorCore.app (Rust $VERSION; set NOTARY_PROFILE to notarize)..."
  codesign --force --deep --sign - "$ROOT/dist/AnchorCore.app"
  codesign --verify --verbose=2 "$ROOT/dist/AnchorCore.app" 2>&1 | head -n 5 || true
fi

# 5. Zip for distribution (Finder shows .app as one file; recipients unzip once)
# Same name as the CI asset (release.yml build-rust-macos).
echo "==> Zipping dist/AnchorCore.app..."
rm -f "$ROOT/dist/AnchorCore-macos.zip"
(cd "$ROOT/dist" && zip -rq AnchorCore-macos.zip AnchorCore.app)

# 6. R41.2: drag-to-Applications DMG + the CI checks (layout, signature, MCP
# handshake against the mounted payload). Shared scripts keep CI and local identical.
echo "==> Packing dist/AnchorCore-macos.dmg..."
bash "$ROOT/scripts/make_macos_dmg.sh" "$ROOT/dist/AnchorCore.app" "$ROOT/dist/AnchorCore-macos.dmg"
if [ -n "$DEVID" ]; then
  codesign --force --timestamp --sign "$DEVID" "$ROOT/dist/AnchorCore-macos.dmg"
  bash "$ROOT/scripts/notarize_macos.sh" "$ROOT/dist/AnchorCore-macos.dmg"
fi
ANCHOR_REQUIRE_NOTARIZED="$([ -n "$DEVID" ] && echo 1 || echo 0)" \
  bash "$ROOT/scripts/verify_macos_dmg.sh" "$ROOT/dist/AnchorCore-macos.dmg"

echo ""
echo "Done: $ROOT/dist/AnchorCore-macos.dmg (+ .zip) (Rust $VERSION, opens http://127.0.0.1:8000)"
echo "Open the DMG, drag AnchorCore.app onto Applications, then open it from /Applications."
echo "First run creates ~/.anchorcore and opens http://127.0.0.1:8000 (set ANCHOR_OPEN_BROWSER=0 to disable)"
