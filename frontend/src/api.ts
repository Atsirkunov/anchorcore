import type { AskResponse, Entity, Health, MergeProposal, Source } from "./types";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `Request failed: ${res.status}`);
  }
  return res.json();
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
    request<{ items: number; entities: number }>(`/sources/${id}/sync`, { method: "POST" }),
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
};
