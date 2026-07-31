import { API_URL } from "./types";
import type { DocumentSummary, KnowledgeObject } from "./types";

export async function listDocuments(): Promise<DocumentSummary[]> {
  const res = await fetch(`${API_URL}/documents`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to list documents: ${res.status}`);
  return res.json();
}

export async function listObjects(documentId: number): Promise<KnowledgeObject[]> {
  const res = await fetch(`${API_URL}/documents/${documentId}/objects`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to list objects: ${res.status}`);
  return res.json();
}

export async function uploadDocument(file: File): Promise<{ id: number }> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`${API_URL}/documents`, { method: "POST", body: form });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `Upload failed: ${res.status}`);
  }
  return res.json();
}
