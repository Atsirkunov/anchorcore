/** Source labels offered in the UI. R35.4: "pii" was bundled into
 * "sensitive" (migration 29); the API rejects it. */
export const LABEL_OPTIONS = [
  { value: "internal", label: "Internal" },
  { value: "public", label: "Public" },
  { value: "sensitive", label: "Sensitive" },
] as const;
