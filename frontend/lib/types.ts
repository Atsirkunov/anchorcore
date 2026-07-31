export const API_URL = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8000";

export type KnowledgeObject = {
  id: number;
  document_id: number;
  kind: "decision" | "document" | "action" | "note";
  summary: string;
  reasoning: string;
  confidence: number;
  author: string;
  source: string;
  status: string;
  created_at: string;
};

export type DocumentSummary = {
  id: number;
  filename: string;
  content_type: string;
  text_length: number;
  created_at: string;
};
