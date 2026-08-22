import type { AppSettings, AskResponse, AskTurn, Dispute, Entity, Health, Job, LogFile, MergeProposal, OnboardingState, PiiChunk, PiiConfig, Project, Source, SystemEvent, SystemStatus, TestConnectionResult } from "./types";

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
  let csrfHeaders: Record<string, string> = {};
  if (isMutating && path !== "/csrf") {
    const token = await getCsrfToken();
    if (token) csrfHeaders["X-CSRF-Token"] = token;
  }
  const mergedInit: RequestInit = {
    ...init,
    headers: { ...(init?.headers as Record<string, string> | undefined), ...csrfHeaders },
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

export const api = {
  health: () => request<Health>("/health"),

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
    request<{ cancelled: boolean; job_id: number }>(`/sources/jobs/${id}/cancel`, { method: "POST" }),
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

  listEntities: (params?: { kind?: string }) =>
    request<Entity[]>(`/entities${params?.kind ? `?kind=${params.kind}` : ""}`),
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
  entityContext: (id: number) =>
    request<{ entity_id: number; source_name: string | null; source_ref: string; window_text: string; expanded_before: string[]; expanded_after: string[]; full_text: string; highlight: string; item_title: string }>(`/entities/${id}/context`),

  lowConfidence: () => request<Entity[]>("/review/low-confidence"),
  duplicates: () => request<MergeProposal[]>("/review/duplicates"),
  decideMerge: (proposalId: number, decision: "merge" | "dismiss") =>
    request<{ merged: boolean; entity_id?: number }>("/review/merge", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ proposal_id: proposalId, decision }),
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
  onboarding: () => request<OnboardingState>("/system/onboarding"),
  getSettings: () => request<AppSettings>("/settings"),
  updateSettings: (payload: Partial<AppSettings>) =>
    request<AppSettings>("/settings", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  testConnection: (provider: "ollama" | "classifier" | "embedder" | "answer") =>
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
  logDownloadUrl: (name: string) => `/system/logs/${encodeURIComponent(name)}`,

  piiConfig: () => request<PiiConfig>("/pii/config"),
  updatePiiConfig: (payload: { custom_words?: string[]; disabled_categories?: string[] }) =>
    request<PiiConfig>("/pii/config", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  piiReview: (params?: { only_flagged?: boolean; limit?: number; offset?: number }) => {
    const q = new URLSearchParams();
    if (params?.only_flagged !== undefined) q.set("only_flagged", String(params.only_flagged));
    if (params?.limit !== undefined) q.set("limit", String(params.limit));
    if (params?.offset !== undefined) q.set("offset", String(params.offset));
    return request<PiiChunk[]>("/pii/review" + (q.toString() ? `?${q}` : ""));
  },
  decidePii: (chunkId: number, is_pii: boolean) =>
    request<PiiChunk>(`/pii/review/${chunkId}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ is_pii }),
    }),
  scanPiiSource: (sourceId: number) =>
    request<{ source_id: number; chunks: number; flagged: number }>(`/pii/scan/${sourceId}`, { method: "POST" }),
};
