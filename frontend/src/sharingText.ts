import type { SensitiveSharing, SharingDestination } from "./types";

/** `https://api.openai.com` → `api.openai.com` (display only). */
export function hostOf(origin: string): string {
  return origin.replace(/^[a-z]+:\/\//i, "");
}

/** R35.6: destinations outside this computer that Sensitive content may
 * reach right now (local providers never count). */
export function sensitiveReach(s: SensitiveSharing): string[] {
  const out: string[] = [];
  for (const d of [s.embed, s.answer]) {
    if (d.allowed && !d.local) out.push(hostOf(d.destination));
  }
  if (s.agents.allowed) out.push("AI agents (MCP / skill)");
  return out;
}

/** Source-row hint for a Sensitive source: "local only" vs "also sent to …". */
export function sensitiveHint(s: SensitiveSharing | undefined): string {
  if (!s) return "local models only · never shared";
  const reach = sensitiveReach(s);
  return reach.length === 0 ? "local models only · never shared" : `also sent to ${reach.join(", ")}`;
}

/** One destination's status line under its switch. */
export function destinationStatus(d: SharingDestination): string {
  if (d.local) return "Local — Sensitive content stays on this computer; no switch needed.";
  if (d.env) return "Set by environment (ANCHOR_CLOUD_TRUST / ANCHOR_TRUSTED_PROVIDERS) — an admin override.";
  if (d.switch) return `On — Sensitive content may be sent to ${d.id === "agents" ? d.destination : hostOf(d.destination)}.`;
  if (d.stale_for) return `Was allowed for ${hostOf(d.stale_for)}; the provider changed, so it is off until you allow the new host.`;
  return "Off — Sensitive content is withheld.";
}

/** react-query key shared by the Settings card and the source rows. */
export const SHARING_QUERY_KEY = ["sensitive-sharing"] as const;
