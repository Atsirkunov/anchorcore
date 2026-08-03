export type Source = {
  id: number;
  connector: string;
  name: string;
  enabled: boolean;
  last_synced_at: string | null;
  created_at: string;
};

export type Entity = {
  id: number;
  item_id: number;
  kind: "decision" | "document" | "action" | "note";
  summary: string;
  reasoning: string;
  confidence: number;
  author: string;
  source_ref: string;
  status: string;
  owner: string;
  created_at: string;
  updated_at: string;
};

export type Citation = {
  entity_id: number;
  kind: string;
  summary: string;
  source_ref: string;
  score: number;
  snippet: string;
};

export type AskResponse = {
  answer: string;
  citations: Citation[];
};

export type MergeProposal = {
  id: number;
  entity_a_id: number;
  entity_b_id: number;
  reason: string;
};
