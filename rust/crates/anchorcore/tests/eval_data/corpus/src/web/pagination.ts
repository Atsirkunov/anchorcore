// Cursor pagination helper: encodes and decodes the opaque page cursor.

const CURSOR_SEP = ":";

/** Encode a row id into a pagination cursor string. */
export function encodeCursor(rowId: string, sortKey: string): string {
  const helper = `${rowId}${CURSOR_SEP}${sortKey}`;
  return Buffer.from(helper, "utf8").toString("base64url");
}

/** Decode a pagination cursor back into its row id and sort key. */
export function decodeCursor(cursor: string): { rowId: string; sortKey: string } {
  const helper = Buffer.from(cursor, "base64url").toString("utf8");
  const [rowId, sortKey] = helper.split(CURSOR_SEP);
  return { rowId, sortKey };
}

// The helper encodes the cursor on every page; the list endpoint passes
// the helper output back as the next request cursor for pagination.
