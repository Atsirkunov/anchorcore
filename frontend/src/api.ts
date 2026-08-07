import type { AppSettings, AskResponse, AskTurn, Dispute, Entity, Health, Job, LogFile, MergeProposal, OnboardingState, Project, Source, SystemEvent, SystemStatus, TestConnectionResult } from "./types";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
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
  createSource: (payload: { connector: string; name: string; config: Record<string, string> }) =>
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
  updateSource: (id: number, payload: { name?: string; enabled?: boolean; config?: Record<string, string> }) =>
    request<Source>(`/sources/${id}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),
  sourceConfig: (id: number) => request<Record<string, string>>(`/sources/${id}/config`),

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

  lowConfidence: () => request<Entity[]>("/review/low-confidence"),
  duplicates: () => request<MergeProposal[]>("/review/duplicates"),
  decideMerge: (proposalId: number, decision: "merge" | "dismiss") =>
    request<{ merged: boolean; entity_id?: number }>("/review/merge", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ proposal_id: proposalId, decision }),
    }),

  ask: (question: string, history: AskTurn[] = [], projectId?: number) =>
    request<AskResponse>("/qa", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        question,
        history: history.map((t) => ({ role: t.role, content: t.content })),
        project_id: projectId ?? null,
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
};
