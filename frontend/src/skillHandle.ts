// R47.4: the search skill installs as `ask-docs` or a handle the person picks
// (its folder and `/name`). The server validates too; this mirrors its rule
// so the confirm step can say what's wrong before the request.

export const DEFAULT_SEARCH_HANDLE = "ask-docs";

const HANDLE_RE = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;

export function handleError(handle: string): string | null {
  const h = handle.trim();
  if (!HANDLE_RE.test(h)) return "Use 1–64 lowercase letters, digits or inner hyphens, like ask-docs.";
  if (h === "anchorcore-setup") return "That's the setup skill's folder — pick another.";
  return null;
}

/** The handle already installed in that agent, else the default. */
export function initialHandle(installed: string[] | undefined): string {
  return installed?.[0] ?? DEFAULT_SEARCH_HANDLE;
}

export type SkillInstallResult = {
  ok: boolean;
  path?: string;
  handle?: string;
  already_installed?: boolean;
  backup?: string | null;
  retired?: string[];
  error?: string;
};

export function installNote(r: SkillInstallResult, fallbackPath: string, clientName: string): string {
  if (!r.ok) return r.error ?? "Install failed";
  const as = r.handle ? ` as /${r.handle}` : "";
  const head = r.already_installed ? `Already installed${as} at` : `Installed${as} to`;
  const backup = r.backup ? " (previous file backed up)" : "";
  const retired = r.retired?.length ? ` Retired ${r.retired.length} older copy${r.retired.length > 1 ? "ies" : ""} (kept as .anchorcore.bak).` : "";
  const restart = r.already_installed ? "" : ` Restart ${clientName} to pick it up.`;
  return `${head} ${r.path ?? fallbackPath}${backup}.${retired}${restart}`;
}
