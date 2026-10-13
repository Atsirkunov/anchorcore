/** Source labels offered in the UI. "pii" was removed 2026-10-13 (owner decision);
 * the backend still accepts it, so LabelSelect renders a legacy fallback for it. */
export const LABEL_OPTIONS = [
  { value: "internal", label: "Internal" },
  { value: "public", label: "Public" },
  { value: "sensitive", label: "Sensitive" },
] as const;
