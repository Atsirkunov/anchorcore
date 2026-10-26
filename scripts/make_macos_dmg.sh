#!/usr/bin/env bash
# R41.2: pack an already-signed AnchorCore.app into a drag-to-Applications DMG.
# Shared by build.sh (local) and release.yml build-rust-macos (CI) so both
# produce the identical layout:
#   /Volumes/AnchorCore/AnchorCore.app
#   /Volumes/AnchorCore/Applications -> /Applications
# Usage: scripts/make_macos_dmg.sh <path/to/AnchorCore.app> <out.dmg>
# Sign the .app BEFORE calling this: the DMG carries the bundle byte-for-byte
# (ditto keeps the signature, xattrs and symlinks); the DMG itself is unsigned.
set -euo pipefail

[ "$(uname -s)" = "Darwin" ] || { echo "make_macos_dmg.sh: macOS only (needs hdiutil)" >&2; exit 1; }
[ $# -eq 2 ] || { echo "usage: $0 <AnchorCore.app> <out.dmg>" >&2; exit 2; }
APP="$1"
OUT="$2"
[ -d "$APP/Contents/MacOS" ] || { echo "FAULT: $APP is not an app bundle" >&2; exit 1; }
codesign --verify --deep --strict "$APP" || { echo "FAULT: $APP is not validly signed; sign before packing" >&2; exit 1; }

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/anchorcore-dmg.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/AnchorCore.app"
ln -s /Applications "$STAGE/Applications"

rm -f "$OUT"
# hdiutil create intermittently fails with "Resource busy" on CI runners; retry.
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname AnchorCore -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$OUT" >/dev/null; then
    break
  fi
  [ "$attempt" -eq 5 ] && { echo "FAULT: hdiutil create failed 5 times" >&2; exit 1; }
  echo "hdiutil create failed (attempt $attempt), retrying..." >&2
  sleep $((attempt * 2))
done
hdiutil verify "$OUT" >/dev/null
echo "ok: $OUT ($(du -h "$OUT" | cut -f1 | tr -d ' '))"
