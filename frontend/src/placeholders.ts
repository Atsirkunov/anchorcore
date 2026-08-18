/** Shared secret-placeholder handling (B36).

The UI masks stored secrets as the literal string `***set***` (both the
Settings and Sources tabs do). When submitting a payload back to the backend
that string must never be sent as a real value — it would overwrite the stored
secret with the placeholder text. `stripPlaceholders` removes those keys so the
backend keeps the stored secret untouched.
 */

export const PLACEHOLDER = "***set***";

/** Returns a copy of `payload` with any `***set***` values removed. */
export function stripPlaceholders<T extends Record<string, unknown>>(payload: T): Partial<T> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(payload)) {
    if (typeof value === "string" && value === PLACEHOLDER) continue;
    out[key] = value;
  }
  return out as Partial<T>;
}
