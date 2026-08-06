export type AppSettings = {
  ollama_base_url: string;
  classifier_model: string;
  embed_model: string;
  classifier_timeout: string;
  classifier_base_url: string;
  embed_base_url: string;
  answer_model: string;
  answer_base_url: string;
  answer_timeout: string;
  answer_reasoning_effort: string; // none | low | medium | high
  answer_api_key: string; // masked as "***set***" when stored
  classifier_api_key: string; // masked as "***set***" when stored
  embed_api_key: string; // masked as "***set***" when stored
};

export type TestConnectionResult = {
  ok: boolean;
  provider: string;
  message: string;
};

export type Source = {
  id: number;
  connector: string;
  name: string;
  enabled: boolean;
  last_synced_at: string | null;
  last_error: string | null;
  error_count: number;
  created_at: string;
};

export type ClassifierStats = {
  windows: number;
  avg_latency_ms: number;
  concurrency: number;
};

export type Health = {
  status: string;
  data_dir: string;
  components: {
    ollama: string;
    answer_key: string;
    pending_embeddings: number;
    tasks: Record<string, string>;
    classifier: ClassifierStats;
    failing_sources: { id: number; name: string; error: string | null; count: number }[];
  };
};

export type ClassifierStatus = ClassifierStats & {
  provider: "local" | "cloud";
  base_url: string;
  model: string;
};

export type EmbedderStatus = {
  provider: "local" | "cloud";
  base_url: string;
  model: string;
};

export type SystemStatus = {
  version: string;
  data_dir: string;
  database: string;
  ollama: {
    reachable: boolean;
    base_url: string;
    classifier_model: string;
    embed_model: string;
    missing_models: string[];
  };
  classifier: ClassifierStatus;
  embedder: EmbedderStatus;
  answer: { provider: string; model: string; base_url: string };
  tasks: Record<string, string>;
  pending_embeddings: number;
  failing_sources: { id: number; name: string; error: string | null; count: number }[];
};

export type SystemEvent = {
  id: number;
  component: string;
  level: "error" | "warning" | "info";
  source_id: number | null;
  source_name: string | null;
  message: string;
  detail: string;
  created_at: string;
};

export type LogFile = {
  name: string;
  size: number;
  modified: string;
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
  window_text: string;
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

export type AskTurn = {
  role: "user" | "assistant";
  content: string;
  citations?: Citation[];
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

export type Job = {
  id: number;
  source_id: number;
  kind: "sync" | "reclassify";
  status: "running" | "done" | "failed" | "cancelled";
  total: number;
  processed: number;
  result: { items?: number; entities?: number };
  error: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
};
