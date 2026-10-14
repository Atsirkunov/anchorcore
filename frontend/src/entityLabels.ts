/** R31.2: plain-language labels for the Entities view.
 *
 * The backend taxonomy is an OPEN set: R33.6 buckets land as
 * `record`/`reference`/`code` doc_types with `decision`/`action`/`note` kinds,
 * but old rows keep their legacy 8-way doc_type (`documentation`, `runbook`,
 * …) and `document` kinds until re-sync. Every helper here passes unknown
 * values through — never assume a closed set.
 */

function titleize(raw: string): string {
  return raw
    .replace(/_/g, " ")
    .replace(/^./, (c) => c.toUpperCase());
}

/** Entity kind → plain label. Unknown kinds pass through titleized. */
export function kindLabel(kind: string): string {
  switch (kind) {
    case "decision":
      return "Decision";
    case "action":
      return "To-do";
    case "note":
      return "Note";
    case "document":
      return "Document";
    default:
      return titleize(kind);
  }
}

/** Parent item doc_type → plain badge label. Null when the backend sent none.
 *  Any string renders — legacy 8-way values pass through titleized. */
export function docTypeLabel(docType: string | undefined | null): string | null {
  if (!docType) return null;
  switch (docType) {
    case "record":
      return "Record";
    case "reference":
      return "Reference";
    case "code":
      return "Code";
    default:
      return titleize(docType);
  }
}

/** Trust status → honest plain label. "Verified" used to claim a human check
 *  that ingest auto-verify never performed — the new labels state the
 *  consequence (in answers or not) instead. Unknown statuses pass through. */
export function statusLabel(status: string): string {
  switch (status) {
    case "unverified":
      return "New";
    case "verified":
      return "In answers";
    case "disputed":
      return "Disputed";
    default:
      return titleize(status);
  }
}

/** The single attention queue: never looked at, or actively contested. */
export function needsLook(status: string): boolean {
  return status === "unverified" || status === "disputed";
}

/** Last path segment (stable doc title fallback). */
export function basename(ref: string): string {
  const parts = ref.split(/[/\\]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : ref;
}

/** Provenance refs stay verbatim when relative; absolute machine paths
 *  collapse to their last two segments (no /Users/… noise, no layout leak). */
export function shortRef(ref: string): string {
  if (!/^([A-Za-z]:\\|\\\\|\/)/.test(ref)) return ref;
  const parts = ref.split(/[/\\]/).filter(Boolean);
  return parts.length > 1 ? parts.slice(-2).join("/") : ref;
}
