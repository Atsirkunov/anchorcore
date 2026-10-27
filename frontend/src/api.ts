import type { AppSettings, AskResponse, AskTurn, BackupExportOptions, BackupImportOptions, BackupImportReport, CancelJobResult, ConflictLink, Dispute, Entity, FolderListing, Health, Job, LogFile, McpClient, McpInfo, MergeProposal, OnboardingState, Project, PullProgress, Source, SystemEvent, SystemStatus, TestConnectionResult, UpdateStatus } from "./types";

export class RequestAbortedError extends Error {
  constructor(message = "Request cancelled") {
    super(message);
    this.name = "RequestAbortedError";
  }
}

const REQUEST_TIMEOUT_MS = 30_000;

// R7.1: per-session CSRF token for local-mode mutating requests
let csrfToken: string | null = null;
async function getCsrfToken(): Promise<string | null> {
  if (csrfToken !== null) return csrfToken;
  try {
    const res = await fetch("/csrf", { method: "GET" });
    if (res.ok) {
      const data = await res.json();
      csrfToken = data.csrf_token ?? null;
      return csrfToken;
    }
  } catch {
    // ignore
  }
  return null;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  // an external signal (e.g. a Cancel button) wins over the timeout
  const onExternalAbort = () => controller.abort();
  if (init?.signal) {
    if (init.signal.aborted) controller.abort();
    else init.signal.addEventListener("abort", onExternalAbort, { once: true });
  }
  // R7.1: attach CSRF token for mutating requests when auth is off (local mode)
  const method = (init?.method ?? "GET").toUpperCase();
  const isMutating = method === "POST" || method === "PUT" || method === "PATCH" || method === "DELETE";
  const csrfHeaders: Record<string, string> = {};
  if (isMutating && path !== "/csrf") {
    const token = await getCsrfToken();
    if (token) (csrfHeaders as Record<string, string>)["X-CSRF-Token"] = token;
  }
  const mergedInit: RequestInit = {
    ...init,
    // R35.5: the app identifies itself so the Sensitive gate exempts the
    // person at the UI (agents send no such header and stay gated).
    headers: { "X-Anchor-Client": "ui", ...(init?.headers as Record<string, string> | undefined), ...csrfHeaders },
    signal: controller.signal,
  };
  let res: Response;
  try {
    res = await fetch(path, mergedInit);
  } catch (e) {
    if (controller.signal.aborted) throw new RequestAbortedError();
    throw e;
  } finally {
    window.clearTimeout(timeout);
    if (init?.signal) init.signal.removeEventListener("abort", onExternalAbort);
  }
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
  return res.json();
}

async function errorMessage(res: Response): Promise<string> {
  const text = (await res.text()).trim();
  if (!text) return `Request failed: ${res.status}`;
  try {
    const parsed = JSON.parse(text);
    if (typeof parsed.detail === "string") return parsed.detail;
    if (Array.isArray(parsed.detail)) {
      return parsed.detail.map((d: { msg?: string }) => d.msg ?? JSON.stringify(d)).join("; ");
    }
  } catch {
    // not JSON — fall through
  }
  return text.slice(0, 500);
}

/** R35.5: raw bytes with the UI client header (`<img>` can't set headers,
 *  so thumbnails fetch through here and render a blob URL). */
async function requestBlob(path: string): Promise<Blob> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  let res: Response;
  try {
    res = await fetch(path, { headers: { "X-Anchor-Client": "ui" }, signal: controller.signal });
  } catch (e) {
    if (controller.signal.aborted) throw new RequestAbortedError();
    throw e;
  } finally {
    window.clearTimeout(timeout);
  }
  if (!res.ok) {
    throw new Error(await errorMessage(res));
  }
  return res.blob();
}

/** R45.9: filters → query string, dropping empty values. */
function recordQuery(f: Record<string, string | number | boolean | undefined>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(f)) {
    if (v === undefined || v === "" || v === false) continue;
    q.set(k, String(v));
  }
  const qs = q.toString();
  return qs ? `?${qs}` : "";
}

export const api = {
  health: () => request<Health>("/health"),

  // R45.9: decision log + action tracker (same contract as the MCP tools).
  listDecisions: (f: import("./types").DecisionFilters = {}) =>
    request<import("./types").RecordListing<"decisions", import("./types").DecisionRecord>>(`/decisions${recordQuery(f)}`),
  listActions: (f: import("./types").ActionFilters = {}) =>
    request<import("./types").RecordListing<"actions", import("./types").ActionRecord>>(`/actions${recordQuery(f)}`),

  listSources: () => request<Source[]>("/sources"),
  createSource: (payload: { connector: string; name: string; config: Record<string, string>; label?: string }) =>
    request<Source>("/sources", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  syncSource: (id: number) =>
    request<Job>(`/sources/${id}/sync`, { method: "POST" }),
  reclassifySource: (id: number) =>
    request<Job>(`/sources/${id}/reclassify`, { method: "POST" }),
  getJob: (id: number) => request<Job>(`/sources/jobs/${id}`),
  cancelJob: (id: number) =>
    request<CancelJobResult>(`/sources/jobs/${id}/cancel`, { method: "POST" }),
  listJobs: (sourceId?: number, limit = 10) => {
    const params = new URLSearchParams({ limit: String(limit) });
    if (sourceId !== undefined) params.set("source_id", String(sourceId));
    return request<Job[]>(`/sources/jobs?${params}`);
  },
  runningJobs: () => request<Job[]>("/sources/jobs/running"),
  deleteSource: (id: number) => request<{ deleted: boolean }>(`/sources/${id}`, { method: "DELETE" }),
  updateSource: (id: number, payload: { name?: string; enabled?: boolean; config?: Record<string, string>; label?: string }) =>
    request<Source>(`/sources/${id}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  sourceConfig: (id: number) => request<Record<string, string>>(`/sources/${id}/config`),
  listJiraProjects: (payload: { base_url: string; email: string; token: string }) =>
    request<{ key: string; name: string }[]>("/sources/jira/projects", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  previewRest: (config: Record<string, string>) =>
    request<{ count: number; truncated: boolean; docs: { external_id: string; title: string; text_preview: string; author: string; updated_at: string; source_ref: string }[] }>(
      "/sources/rest/preview",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ config }),
      },
    ),

  listProjects: () => request<Project[]>("/projects"),
  createProject: (payload: { name: string; source_ids?: number[] }) =>
    request<Project>("/projects", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  updateProject: (id: number, payload: { name?: string; is_default?: boolean; source_ids?: number[] }) =>
    request<Project>(`/projects/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  deleteProject: (id: number) => request<{ deleted: boolean }>(`/projects/${id}`, { method: "DELETE" }),
  defaultProject: () => request<Project | null>("/projects/default"),

  listEntities: (params?: { kind?: string; status?: string; limit?: number }) => {
    const q = new URLSearchParams();
    if (params?.kind) q.set("kind", params.kind);
    if (params?.status) q.set("status", params.status);
    if (params?.limit) q.set("limit", String(params.limit));
    const qs = q.toString();
    return request<Entity[]>(`/entities${qs ? `?${qs}` : ""}`);
  },
  getEntity: (id: number) => request<Entity>(`/entities/${id}`),
  updateEntity: (id: number, payload: Partial<Pick<Entity, "kind" | "status" | "owner">>) =>
    request<Entity>(`/entities/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  disputeEntity: (id: number, reason: string) =>
    request<Entity>(`/entities/${id}/dispute`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ reason }),
    }),
  entityDisputes: (id: number) => request<Dispute[]>(`/entities/${id}/disputes`),
  listConflicts: (id: number) => request<ConflictLink[]>(`/entities/${id}/conflicts`),
  flagConflict: (id: number, otherId: number, note?: string) =>
    request<{ id: number; from_entity_id: number; to_entity_id: number; kind: string; note: string; created_at: string }>(`/entities/${id}/conflicts`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ other_entity_id: otherId, note }),
    }),
  unflagConflict: (id: number, otherId: number) =>
    request<{ deleted: boolean }>(`/entities/${id}/conflicts/${otherId}`, { method: "DELETE" }),
  entityContext: (id: number) =>
    request<{ entity_id: number; source_name: string | null; source_ref: string; window_text: string; expanded_before: string[]; expanded_after: string[]; full_text: string; highlight: string; item_title: string }>(`/entities/${id}/context`),

  // R31.2: the Review tab folded into Entities, but /review/* stays — agents
  // and the folded duplicates queue use it. No backend change.
  lowConfidence: () => request<Entity[]>("/review/low-confidence"),
  duplicates: () => request<MergeProposal[]>("/review/duplicates"),
  decideMerge: (proposalId: number, decision: "merge" | "dismiss") =>
    request<{ merged: boolean; entity_id?: number }>("/review/merge", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ proposal_id: proposalId, decision }),
    }),
  // R45.13: people-owned glossary (replaces the "Words to confirm" queue).
  glossary: () => request<import("./types").GlossaryEntry[]>("/glossary"),
  addGlossaryEntry: (term: string, expansion: string) =>
    request<import("./types").GlossaryEntry>("/glossary", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ term, expansion }),
    }),
  deleteGlossaryEntry: (id: number) => request<{ deleted: number }>(`/glossary/${id}`, { method: "DELETE" }),
  setItemGlossary: (itemId: number, isGlossary: boolean) =>
    request<{ id: number; is_glossary: boolean }>(`/items/${itemId}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ is_glossary: isGlossary }),
    }),

  ask: (question: string, history: AskTurn[] = [], projectId?: number, signal?: AbortSignal, publicOnly = false) =>
    request<AskResponse>(publicOnly ? "/qa/public" : "/qa", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal,
      body: JSON.stringify({
        question,
        history: history.map((t) => ({ role: t.role, content: t.content })),
        project_id: projectId ?? null,
        public_only: publicOnly,
      }),
    }),

  systemStatus: () => request<SystemStatus>("/system/status"),
  /** R47.0: redacted bundle for the last query ("it's wrong" flow). */
  getReport: (index = 0) =>
    request<import("./types").ReportBundle>(`/system/report${index > 0 ? `?index=${index}` : ""}`),
  /** R29.1: `refresh` performs the GitHub request; without it this only reads the cache. */
  updateStatus: (refresh = false) => request<UpdateStatus>(`/system/update${refresh ? "?refresh=1" : ""}`),
  onboarding: () => request<OnboardingState>("/system/onboarding"),
  getSettings: () => request<AppSettings>("/settings"),
  updateSettings: (payload: Partial<AppSettings>) =>
    request<AppSettings>("/settings", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  testConnection: (provider: "ollama" | "embedder" | "answer") =>
    request<TestConnectionResult>("/settings/test-connection", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ provider }),
    }),
  systemErrors: (params?: { component?: string; level?: string; limit?: number }) => {
    const q = new URLSearchParams();
    if (params?.component) q.set("component", params.component);
    if (params?.level) q.set("level", params.level);
    if (params?.limit) q.set("limit", String(params.limit));
    return request<SystemEvent[]>(`/system/errors?${q}`);
  },
  systemLogs: () => request<LogFile[]>("/system/logs"),
  systemMcp: () => request<McpInfo>("/system/mcp"),
  browseFolders: (path?: string) => {
    const q = new URLSearchParams();
    if (path?.trim()) q.set("path", path.trim());
    return request<FolderListing>(`/system/fs${q.toString() ? `?${q}` : ""}`);
  },
  mcpClients: () => request<{ clients: McpClient[] }>("/system/mcp/clients"),
  mcpConnect: (client: string) =>
    request<{ ok: boolean; method?: string; config_path?: string; backup?: string | null; removed?: boolean; error?: string }>("/system/mcp/connect", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ client }),
    }),
  mcpDisconnect: (client: string) =>
    request<{ ok: boolean; method?: string; config_path?: string; removed?: boolean; error?: string }>("/system/mcp/disconnect", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ client }),
    }),
  logDownloadUrl: (name: string) => `/system/logs/${encodeURIComponent(name)}`,
  skillUrl: (name: string) => `/system/skills/${encodeURIComponent(name)}`,
  fetchSkill: async (name: string): Promise<string> => {
    const res = await fetch(`/system/skills/${encodeURIComponent(name)}`);
    if (!res.ok) throw new Error(await errorMessage(res));
    return res.text();
  },
  skillInstallTargets: () =>
    request<{
      skills: { name: string; dir: string; file: string }[];
      clients: { id: string; name: string; detected: boolean; skills_dir: string | null }[];
    }>("/system/skills/install-targets"),
  skillInstall: (name: string, client: string) =>
    request<{ ok: boolean; path?: string; already_installed?: boolean; backup?: string | null; error?: string }>(
      `/system/skills/${encodeURIComponent(name)}/install`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ client }),
      }
    ),

  // R25.5: backup & restore. These bypass request() — an archive is a blob
  // download and an import is a multipart upload that may run for minutes.
  backupExport: async (payload: BackupExportOptions): Promise<{ blob: Blob; filename: string }> => {
    const token = await getCsrfToken();
    const res = await fetch("/backup/export", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...(token ? { "X-CSRF-Token": token } : {}) },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error(await errorMessage(res));
    const disposition = res.headers.get("content-disposition") ?? "";
    const filename = /filename="([^"]+)"/.exec(disposition)?.[1] ?? "anchorcore-backup.anchorcore-backup";
    return { blob: await res.blob(), filename };
  },
  backupImport: async (file: File, options: BackupImportOptions): Promise<BackupImportReport> => {
    const form = new FormData();
    form.append("file", file);
    form.append("options", JSON.stringify(options));
    const token = await getCsrfToken();
    const res = await fetch("/backup/import", {
      method: "POST",
      headers: token ? { "X-CSRF-Token": token } : {},
      body: form,
    });
    if (!res.ok) throw new Error(await errorMessage(res));
    return res.json();
  },

  startOllama: () => request<{ ok: boolean; already_running?: boolean; launched?: boolean; error?: string }>("/system/ollama/start", { method: "POST" }),
  pullModels: (models: string[]) =>
    request<{ started: string[]; already_running?: string[] }>("/system/ollama/pull", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ models }),
    }),
  ollamaPulls: () => request<{ pulls: PullProgress[] }>("/system/ollama/pulls"),
  embedModel: () => request<import("./types").EmbedModelStatus>("/system/embed-model"),
  downloadEmbedModel: () =>
    request<{ started: boolean; ready: boolean }>("/system/embed-model/download", { method: "POST" }),

  sections: (params?: { item_id?: number; source_id?: number }) => {
    const q = new URLSearchParams();
    if (params?.item_id) q.set("item_id", String(params.item_id));
    if (params?.source_id) q.set("source_id", String(params.source_id));
    const qs = q.toString();
    return request<import("./types").Section[]>(`/sections${qs ? `?${qs}` : ""}`);
  },
  sectionChunks: (id: number) => request<import("./types").ChunkInfo[]>(`/sections/${id}/chunks`),
  tags: () => request<import("./types").Tag[]>("/tags"),
  tagChunks: (id: number) => request<import("./types").ChunkInfo[]>(`/tags/${id}/chunks`),
  /** R46.12: images by attribution (`none` = unplaced; `total` is the badge). */
  media: (params?: { attribution?: string; source_id?: number; limit?: number; offset?: number }) => {
    const q = new URLSearchParams();
    if (params?.attribution) q.set("attribution", params.attribution);
    if (params?.source_id) q.set("source_id", String(params.source_id));
    if (params?.limit) q.set("limit", String(params.limit));
    if (params?.offset) q.set("offset", String(params.offset));
    const qs = q.toString();
    return request<import("./types").MediaList>(`/media${qs ? `?${qs}` : ""}`);
  },
  /** R46.12: a person's caption / target / ignore for one image. */
  patchMedia: (assetId: string, patch: import("./types").MediaPatch) =>
    request<import("./types").MediaRow>(`/media/${encodeURIComponent(assetId)}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(patch),
    }),
  /** R35.5: one image's bytes (UI client header — Sensitive images stay
   *  visible to the person at the UI). */
  assetBlob: async (assetId: string): Promise<Blob> => requestBlob(`/media/${encodeURIComponent(assetId)}`),

  /** R46.11: one item with its `assets` (image refs; bytes via /media). */
  item: (itemId: number) => request<import("./types").ItemDetail>(`/items/${itemId}`),
  listItems: (params?: { path_prefix?: string; source_id?: number; limit?: number; offset?: number }) => {
    const q = new URLSearchParams();
    if (params?.path_prefix) q.set("path_prefix", params.path_prefix);
    if (params?.source_id) q.set("source_id", String(params.source_id));
    if (params?.limit) q.set("limit", String(params.limit));
    if (params?.offset) q.set("offset", String(params.offset));
    const qs = q.toString();
    return request<import("./types").ItemList>(`/items${qs ? `?${qs}` : ""}`);
  },
};
