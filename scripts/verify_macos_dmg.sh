#!/usr/bin/env bash
# R41.2 + R20.5: mount AnchorCore-macos.dmg read-only, assert the
# drag-to-Applications layout and the ad-hoc signature, run the stdio MCP
# handshake (initialize + tools/list, 6 tools incl. search) against the binary
# inside the DMG payload, then detach. Used by release.yml and build.sh.
# Usage: scripts/verify_macos_dmg.sh <AnchorCore-macos.dmg>
set -euo pipefail

[ "$(uname -s)" = "Darwin" ] || { echo "verify_macos_dmg.sh: macOS only (needs hdiutil)" >&2; exit 1; }
[ $# -eq 1 ] || { echo "usage: $0 <AnchorCore-macos.dmg>" >&2; exit 2; }
DMG="$1"
MNT="$(mktemp -d "${TMPDIR:-/tmp}/anchorcore-mnt.XXXXXX")"
OUTF="$(mktemp "${TMPDIR:-/tmp}/anchorcore-mcp.XXXXXX")"
cleanup() {
  hdiutil detach "$MNT" -quiet 2>/dev/null || hdiutil detach "$MNT" -force -quiet 2>/dev/null || true
  rmdir "$MNT" 2>/dev/null || true
  rm -f "$OUTF"
  [ -n "${DATA:-}" ] && rm -rf "$DATA"
  return 0
}
trap cleanup EXIT

hdiutil attach "$DMG" -readonly -nobrowse -noautoopen -mountpoint "$MNT" -quiet

# Layout: exactly AnchorCore.app + Applications -> /Applications (hidden
# volume metadata such as .fseventsd is ignored).
echo "DMG contents:"
ls -la "$MNT"
[ -d "$MNT/AnchorCore.app" ] || { echo "FAULT: AnchorCore.app missing from DMG"; exit 1; }
[ -L "$MNT/Applications" ] || { echo "FAULT: Applications symlink missing from DMG"; exit 1; }
[ "$(readlink "$MNT/Applications")" = "/Applications" ] || { echo "FAULT: Applications symlink points at $(readlink "$MNT/Applications")"; exit 1; }
EXTRA="$(ls -1 "$MNT" | grep -vxE 'AnchorCore\.app|Applications' || true)"
[ -z "$EXTRA" ] || { echo "FAULT: unexpected DMG entries: $EXTRA"; exit 1; }

BIN="$MNT/AnchorCore.app/Contents/MacOS/AnchorCore"
[ -x "$BIN" ] || { echo "FAULT: app binary missing or not executable in DMG"; exit 1; }

# The signature survives packing.
codesign --verify --deep --strict --verbose=2 "$MNT/AnchorCore.app"
codesign -dv "$MNT/AnchorCore.app" 2>&1 | grep -E '^(Identifier|Signature|Format|Authority|TeamIdentifier)=' || true

# R41.4: a notarized release (ANCHOR_REQUIRE_NOTARIZED=1, set by release.yml
# when the signing secrets exist) must be Developer ID signed with the
# hardened runtime, carry stapled tickets on the DMG and the app, and pass
# Gatekeeper's assessment — what a fresh download's double-click runs.
if [ "${ANCHOR_REQUIRE_NOTARIZED:-0}" = "1" ]; then
  SIG="$(codesign -dv --verbose=4 "$MNT/AnchorCore.app" 2>&1)"
  grep -q '^Authority=Developer ID Application:' <<<"$SIG" || { echo "FAULT: app is not Developer ID signed"; echo "$SIG"; exit 1; }
  grep -qE '^CodeDirectory .*flags=.*runtime' <<<"$SIG" || { echo "FAULT: app is not signed with the hardened runtime"; echo "$SIG"; exit 1; }
  # stapler validate asks Apple's ticket service; retry transient network drops.
  for T in "$DMG" "$MNT/AnchorCore.app"; do
    for attempt in 1 2 3 4 5; do
      xcrun stapler validate "$T" && break
      [ "$attempt" -eq 5 ] && { echo "FAULT: stapler validate failed 5 times for $T"; exit 1; }
      echo "stapler validate failed (attempt $attempt), retrying..." >&2
      sleep $((attempt * 5))
    done
  done
  spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
  spctl --assess --type execute --verbose=2 "$MNT/AnchorCore.app"
  echo "ok (macos dmg): Developer ID + hardened runtime, tickets stapled, Gatekeeper accepts DMG + app"
fi

# R20.5 MCP handshake against the DMG payload (throwaway data dir so a dev
# machine's ~/.anchorcore is never touched; tools/list needs no server).
DATA="$(mktemp -d "${TMPDIR:-/tmp}/anchorcore-mcpdata.XXXXXX")"
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | ANCHOR_DATA_DIR="$DATA" ANCHOR_SECRETS_NO_KEYRING=1 "$BIN" mcp > "$OUTF" || echo "note: mcp exited $? after stdin closed (output is what is asserted)"
for TOOL in ask search get_entity get_source list_sources memory_status; do
  grep -q "\"name\":\"$TOOL\"" "$OUTF" || { echo "FAULT: MCP tool '$TOOL' missing from tools/list (DMG payload)"; cat "$OUTF"; exit 1; }
done
echo "ok (macos dmg): layout app + /Applications link, signature valid, stdio MCP server lists 6 tools incl. search"
