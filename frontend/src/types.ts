export type AppSettings = {
  ollama_base_url: string;
  embed_model: string;
  embed_timeout?: string;
  embed_base_url: string;
  answer_model: string;
  answer_base_url: string;
  answer_timeout: string;
  answer_reasoning_effort: string; // none | low | medium | high
  answer_api_key: string; // masked as "***set***" when stored
  embed_api_key: string; // masked as "***set***" when stored
  tag_reuse_threshold?: string;
  update_check_enabled?: string; // "1" on, "0"/"" off (startup update check)
  llm_mode?: LlmMode; // R45.2: who writes answers (default "local")
};

/** R45.2: `agent` — your AI agent (MCP or the REST skill) writes answers and the
 * server runs no chat model; `local` — the configured answer model does. */
export type LlmMode = "agent" | "local";

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
  /** "pii" is deprecated (hidden from the UI since 2026-10-13) but the backend still accepts it for existing sources. */
  label: "internal" | "public" | "sensitive" | "pii";
  last_synced_at: string | null;
  last_error: string | null;
  error_count: number;
  created_at: string;
};

export type Project = {
  id: number;
  name: string;
  is_default: boolean;
  created_at: string;
  source_ids: number[];
};

export type Health = {
  status: string;
  data_dir: string;
  components: {
    ollama: string;
    answer_key: string;
    missing_models: string[];
    answer_ready: boolean;
    embedder?: { provider: string };
    pending_embeddings: number;
    tasks: Record<string, string>;
    failing_sources: { id: number; name: string; error: string | null; count: number }[];
  };
};

export type RetrievalStatus = {
  calls: number;
  vec0_calls: number;
  avg_latency_ms: number;
  backend: "vec0" | "scan" | "none";
};

export type EmbedderStatus = {
  provider: "local" | "cloud";
  base_url: string;
  model: string;
};

export type OnboardingState = {
  needs_wizard: boolean;
  sources_count: number;
  ollama: {
    reachable: boolean;
    base_url: string;
    missing_models: string[];
  };
  answer_provider: "ollama" | "configured" | "missing";
  answer_ready: boolean;
  sample: {
    available: boolean;
    path: string | null;
  };
};

export type SystemStatus = {
  version: string;
  data_dir: string;
  database: string;
  ollama: {
    reachable: boolean;
    base_url: string;
    embed_model: string;
    missing_models: string[];
  };
  /** R45.18: entity extraction is model-free — record-structure rules, or off. */
  extraction: { mode: "rules" | "off" };
  retrieval: RetrievalStatus;
  embedder: EmbedderStatus;
  llm_mode?: LlmMode;
  answer: { provider: string; model: string; base_url: string; mode?: LlmMode };
  answer_ready: boolean;
  tasks: Record<string, string>;
  pending_embeddings: number;
  failing_sources: { id: number; name: string; error: string | null; count: number }[];
};

/** R29.1: `GET /system/update` — the cached release check. A plain GET never
 * contacts github.com; `error` is a plain-language reason when a check failed. */
export type UpdateStatus = {
  current: string;
  latest: string | null;
  update_available: boolean;
  release_url: string | null;
  published_at: string | null;
  asset_url: string | null;
  checked_at: string | null;
  check_enabled: boolean;
  error: string | null;
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

export type McpInfo = {
  mode: string;
  command: string | null;
  args: string[];
  calls: number;
  last_call_at: string | null;
};

export type McpClient = {
  id: string;
  name: string;
  detected: boolean;
  connected: boolean;
  method: string;
  config_path: string | null;
};

export type FolderEntry = {
  name: string;
  path: string;
};

/** GET /system/fs — one directory level for the folder picker. `path`/`parent`
 * are null in the shortcut (home + drive roots) view. */
export type FolderListing = {
  path: string | null;
  parent: string | null;
  home: string | null;
  dirs: FolderEntry[];
  truncated: boolean;
};

export type Entity = {
  id: number;
  item_id: number;
  /** Open set: decision/action/note (+ legacy document rows) and anything else
   *  the backend sends — render via entityLabels.kindLabel, never a closed union. */
  kind: string;
  summary: string;
  reasoning: string;
  confidence: number;
  author: string;
  source_ref: string;
  status: string;
  owner: string;
  window_text: string;
  dispute_count: number;
  doc_type?: string;
  /** @deprecated PII marking was removed from the UI (owner decision 2026-10-13); the backend still sends this for compat. Optional — review endpoints don't send it. */
  item_is_pii?: boolean;
  /** R32.13: the entity's own chunk (first by id); null when none stored. */
  chunk_id?: number | null;
  created_at: string;
  updated_at: string;
};

/** R32.11: ingested-item metadata from GET /items (no bytes — drill down via GET /items/:id). */
export type ItemMeta = {
  id: number;
  source_id: number;
  title: string;
  path: string;
  /** @deprecated PII marking was removed from the UI (owner decision 2026-10-13); the backend still sends this for compat. */
  is_pii: boolean;
  /** R45.13: the file is used as a glossary (its rows teach search). */
  is_glossary?: boolean;
};

export type ItemList = {
  items: ItemMeta[];
  total: number;
};

export type ConflictLink = {
  other_entity_id: number;
  note: string;
  created_at: string;
};

export type Dispute = {
  id: number;
  entity_id: number;
  reason: string;
  user: string;
  created_at: string;
};

export type Citation = {
  entity_id: number;
  item_id: number | null;
  kind: string;
  summary: string;
  source_ref: string;
  path?: string;
  tags?: string[];
  doc_type?: string;
  env: string;
  score: number;
  conflicts_with?: number[];
  snippet: string;
  content: string;
  item_title: string;
  content_truncated: boolean;
  line_start?: number;
  line_end?: number;
};

export type Section = {
  id: number;
  item_id: number;
  parent_id: number | null;
  level: number;
  title: string;
  path: string;
  chunk_range: string;
  summary: string;
  line_start?: number;
  line_end?: number;
  kind?: string;
  meta?: Record<string, string> | null;
  created_at: string;
};

export type Tag = {
  id: number;
  name: string;
  description: string;
  count: number;
  created_at: string;
};

export type ChunkInfo = {
  id: number;
  item_id: number | null;
  section_id: number | null;
  kind: string;
  source_ref: string;
  content: string;
  path: string;
  line_start?: number;
  line_end?: number;
  created_at: string;
};

export type AskTurn = {
  role: "user" | "assistant";
  content: string;
  citations?: Citation[];
  mode?: string;
  retrieval?: string;
};

export type AskResponse = {
  answer: string;
  citations: Citation[];
  refusal?: string | null;
  /** "composed" | "agent_compose" (R45.2) | "context_only" | "none" */
  mode?: string;
  retrieval?: string;
  warning?: string | null;
};

export type PullProgress = {
  model: string;
  status: string;
  completed: number;
  total: number;
  done: boolean;
  error: string | null;
};

export type MergeProposal = {
  id: number;
  entity_a_id: number;
  entity_b_id: number;
  reason: string;
};

/** R45.13: a people-owned glossary entry (typed in, or from a marked file). */
export type GlossaryEntry = {
  id: number;
  term: string;
  expansion: string;
  origin: "manual" | "glossary";
  item_id: number | null;
  path: string | null;
};

export type Job = {
  id: number;
  source_id: number;
  kind: "sync" | "reclassify" | "embed";
  status: "running" | "pending" | "done" | "failed" | "cancelled";
  total: number;
  processed: number;
  result: { items?: number; entities?: number; purged?: number; unchanged?: number };
  error: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
};

// R25.5 — backup / restore (Phase A of docs/plan-backup-sync.md)
export type BackupExportOptions = {
  include_secrets?: boolean;
  passphrase?: string;
  include_embeddings?: boolean;
  include_ops?: boolean;
  project_id?: number | null;
};

export type BackupImportOptions = {
  mode?: "merge" | "replace";
  dry_run?: boolean;
  confirm?: string;
  passphrase?: string;
  remap?: Record<string, string>;
};

export type BackupSecretAsk = { source_uid: string; source_name?: string; field: string };
export type BackupFolderPath = { source_uid: string; name: string; path: string };
export type BackupConflict = { type: string; uid: string; action: "imported" | "kept-local" };
export type BackupRemap = { source_uid: string; from: string; to: string };

export type BackupImportReport = {
  dry_run: boolean;
  mode: string;
  format_version: number;
  app_version: string;
  created_at: string;
  counts: Record<string, number>;
  imported: Record<string, number>;
  skipped: Record<string, number>;
  conflicts: BackupConflict[];
  needs_secret: BackupSecretAsk[];
  folder_paths: BackupFolderPath[];
  remap_applied: BackupRemap[];
  secrets_imported: number;
  secrets_note: string | null;
  safety_backup: string | null;
  reembed_source_ids: number[];
  reembed_job_ids?: number[];
  warnings: string[];
};

/** R45.9: provenance carried by every decision/action row. */
export type RecordProvenance = {
  item_id: number;
  source_id: number;
  path: string;
  title: string;
  chunk_id: number | null;
};

/** R45.9: one row of `GET /decisions`. `status` is the effective state —
 *  "superseded" whenever a newer record supersedes this one. */
export type DecisionRecord = {
  entity_id: number;
  record_id: string;
  summary: string;
  status: string;
  current: boolean;
  owner: string;
  supersedes: string[];
  superseded_by: string;
  review_status: string;
  created_at: string;
  provenance: RecordProvenance;
};

/** R45.9: one row of `GET /actions`. */
export type ActionRecord = {
  entity_id: number;
  task: string;
  owner: string;
  due: string;
  done: boolean;
  review_status: string;
  created_at: string;
  provenance: RecordProvenance;
};

/** R45.9: listing envelope; `refusal` is "gated_provider" when every row was withheld. */
export type RecordListing<K extends string, T> = { [key in K]: T[] } & {
  total: number;
  blocked: number;
  refusal: string | null;
};

export type DecisionFilters = { q?: string; status?: string; project_id?: number };
export type ActionFilters = { q?: string; owner?: string; state?: "open" | "done" | "all"; overdue?: boolean; project_id?: number };
