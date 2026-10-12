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
  tag_reuse_threshold?: string;
  update_check_enabled?: string; // "1" on, "0"/"" off (startup update check)
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
    missing_models: string[];
    answer_ready: boolean;
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
    classifier_model: string;
    embed_model: string;
    missing_models: string[];
  };
  classifier: ClassifierStatus;
  retrieval: RetrievalStatus;
  embedder: EmbedderStatus;
  answer: { provider: string; model: string; base_url: string };
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
  kind: "decision" | "document" | "action" | "note";
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
  created_at: string;
  updated_at: string;
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
  kind: string;
  summary: string;
  source_ref: string;
  path?: string;
  tags?: string[];
  doc_type?: string;
  score: number;
  conflicts_with?: number[];
  snippet: string;
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
  mode?: string;
  retrieval?: string;
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

export type Job = {
  id: number;
  source_id: number;
  kind: "sync" | "reclassify" | "embed";
  status: "running" | "pending" | "done" | "failed" | "cancelled";
  total: number;
  processed: number;
  result: { items?: number; entities?: number; purged?: number };
  error: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
};

export type PiiCategory = {
  id: string;
  label: string;
  description: string;
  field_names: string[];
  patterns: string[];
  enabled: boolean;
};

export type PiiConfig = {
  categories: PiiCategory[];
  custom_words: string[];
};

export type PiiMatch = {
  category: string;
  label: string;
  strong: boolean;
  match: string;
};

export type PiiChunk = {
  chunk_id: number;
  source_id: number | null;
  source_name: string;
  source_label: string;
  kind: string;
  content: string;
  snippet: string;
  is_pii: boolean;
  categories: PiiMatch[];
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
