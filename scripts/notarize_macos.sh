#!/usr/bin/env bash
# R41.4: submit a Developer ID-signed file to Apple notarization, wait for the
# verdict, staple the ticket and validate it. Used by release.yml
# build-rust-macos; also runs locally (see docs/releasing.md).
# Usage: scripts/notarize_macos.sh <file-to-submit> [<file-to-staple>]
#   <file-to-submit>  a .zip (an .app goes in as `ditto -c -k --keepParent`) or .dmg
#   <file-to-staple>  defaults to <file-to-submit>; for a zip, pass the .app
# Credentials, either:
#   NOTARY_PROFILE  a keychain profile from `xcrun notarytool store-credentials`
# or an App Store Connect API key:
#   NOTARY_KEY_P8 (path to AuthKey_XXXX.p8) or NOTARY_KEY_P8_BASE64,
#   plus NOTARY_KEY_ID and NOTARY_ISSUER_ID
# On rejection it prints Apple's log for the submission and exits 1.
set -euo pipefail

[ "$(uname -s)" = "Darwin" ] || { echo "notarize_macos.sh: macOS only (needs xcrun)" >&2; exit 1; }
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: $0 <file-to-submit> [<file-to-staple>]" >&2; exit 2; }
SUBMIT="$1"
TARGET="${2:-$1}"
[ -e "$SUBMIT" ] || { echo "FAULT: $SUBMIT not found" >&2; exit 1; }
[ -e "$TARGET" ] || { echo "FAULT: $TARGET not found" >&2; exit 1; }

KEYFILE=""
cleanup() {
  [ -n "$KEYFILE" ] && rm -f "$KEYFILE"
  return 0
}
trap cleanup EXIT
if [ -n "${NOTARY_PROFILE:-}" ]; then
  AUTH=(--keychain-profile "$NOTARY_PROFILE")
else
  KEY="${NOTARY_KEY_P8:-}"
  if [ -z "$KEY" ]; then
    [ -n "${NOTARY_KEY_P8_BASE64:-}" ] || { echo "FAULT: set NOTARY_PROFILE, or NOTARY_KEY_P8[_BASE64] + NOTARY_KEY_ID + NOTARY_ISSUER_ID" >&2; exit 1; }
    KEYFILE="$(mktemp "${TMPDIR:-/tmp}/notary-key.XXXXXX")"
    printf '%s' "$NOTARY_KEY_P8_BASE64" | base64 --decode > "$KEYFILE"
    KEY="$KEYFILE"
  fi
  [ -n "${NOTARY_KEY_ID:-}" ] && [ -n "${NOTARY_ISSUER_ID:-}" ] || { echo "FAULT: NOTARY_KEY_ID and NOTARY_ISSUER_ID are required with an API key" >&2; exit 1; }
  AUTH=(--key "$KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
fi

echo "Submitting $SUBMIT for notarization (usually a few minutes)..."
RC=0
OUT="$(xcrun notarytool submit "$SUBMIT" "${AUTH[@]}" --wait --timeout 45m --output-format json)" || RC=$?
SUB_ID="$(plutil -extract id raw -o - - <<<"$OUT" 2>/dev/null || true)"
STATUS="$(plutil -extract status raw -o - - <<<"$OUT" 2>/dev/null || true)"
if [ "$STATUS" != "Accepted" ]; then
  echo "FAULT: notarization ${STATUS:-failed} (exit $RC, submission ${SUB_ID:-none})" >&2
  echo "$OUT" >&2
  [ -n "$SUB_ID" ] && xcrun notarytool log "$SUB_ID" "${AUTH[@]}" >&2 || true
  exit 1
fi
echo "ok: notarization accepted (submission $SUB_ID)"

# The ticket can lag behind the Accepted verdict on Apple's CDN; retry.
for attempt in 1 2 3 4 5; do
  if xcrun stapler staple "$TARGET"; then
    break
  fi
  [ "$attempt" -eq 5 ] && { echo "FAULT: stapler staple failed 5 times" >&2; exit 1; }
  echo "stapler staple failed (attempt $attempt), retrying..." >&2
  sleep $((attempt * 10))
done
xcrun stapler validate "$TARGET"
echo "ok: stapled $TARGET"
