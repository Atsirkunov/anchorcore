import type { AskResponse, Entity, Health, Job, LogFile, MergeProposal, Source, SystemEvent, SystemStatus } from "./types";

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
  listJobs: (sourceId?: number, limit = 10) => {
    const params = new URLSearchParams({ limit: String(limit) });
    if (sourceId !== undefined) params.set("source_id", String(sourceId));
    return request<Job[]>(`/sources/jobs?${params}`);
  },
  deleteSource: (id: number) => request<{ deleted: boolean }>(`/sources/${id}`, { method: "DELETE" }),

  listEntities: (params?: { kind?: string }) =>
    request<Entity[]>(`/entities${params?.kind ? `?kind=${params.kind}` : ""}`),
  updateEntity: (id: number, payload: Partial<Pick<Entity, "kind" | "status" | "owner">>) =>
    request<Entity>(`/entities/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    }),

  lowConfidence: () => request<Entity[]>("/review/low-confidence"),
  duplicates: () => request<MergeProposal[]>("/review/duplicates"),
  decideMerge: (proposalId: number, decision: "merge" | "dismiss") =>
    request<{ merged: boolean; entity_id?: number }>("/review/merge", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ proposal_id: proposalId, decision }),
    }),

  ask: (question: string) =>
    request<AskResponse>("/qa", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ question }),
    }),

  systemStatus: () => request<SystemStatus>("/system/status"),
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
