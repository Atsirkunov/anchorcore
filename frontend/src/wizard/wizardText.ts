import type { AnswerModelOption, LlmMode } from "../types";

/** R45.6: wizard steps per mode. Local mode picks an answer model and may
 * connect an agent at the end; agent mode connects the agent instead. */
export type StepId = "setup" | "answers" | "agent" | "source" | "ask";

export function stepsFor(mode: LlmMode): StepId[] {
  return mode === "agent" ? ["setup", "agent", "source", "ask"] : ["setup", "answers", "source", "ask", "agent"];
}

export function stepLabel(id: StepId, mode: LlmMode): string {
  switch (id) {
    case "setup":
      return "Mode & search";
    case "answers":
      return "Answer model";
    case "agent":
      return mode === "agent" ? "Connect your agent" : "Connect an agent";
    case "source":
      return "Connect a source";
    case "ask":
      return mode === "agent" ? "Try a search" : "Ask a question";
  }
}

const GIB = 1024 * 1024 * 1024;

/** "16 GB" from bytes, rounded to whole GB (firmware reserves a little). */
export function ramLabel(bytes: number | null | undefined): string | null {
  if (!bytes || bytes <= 0) return null;
  return `${Math.round(bytes / GIB)} GB`;
}

/** True when the model asks for more RAM than this machine has. */
export function exceedsRam(option: AnswerModelOption, bytes: number | null | undefined): boolean {
  if (!bytes || bytes <= 0) return false;
  return option.min_ram_gb > bytes / GIB + 0.5;
}

/** The OpenAI-compatible answer URL of an Ollama base URL. */
export function ollamaAnswerBase(ollamaBase: string): string {
  const base = (ollamaBase.trim() || "http://localhost:11434").replace(/\/+$/, "");
  return base.endsWith("/v1") ? base : `${base}/v1`;
}

/** Mirrors the server rule (ollama.rs `model_installed`): an explicit size
 * tag must match exactly, an untagged or `:latest` name matches its family. */
export function sameModel(a: string, b: string): boolean {
  const norm = (s: string) => s.trim().toLowerCase();
  const [fa, ta] = norm(a).split(":");
  const [fb, tb] = norm(b).split(":");
  if (fa !== fb) return false;
  const loose = (t?: string) => !t || t === "latest";
  return loose(ta) || loose(tb) || ta === tb;
}

/** Whether onboarding's `missing_models` lists this model. */
export function isMissing(missing: string[], model: string): boolean {
  return missing.some((m) => sameModel(m, model));
}

/** Preselect: the configured local answer model when the catalog has it,
 * else the server's RAM-sized recommendation. */
export function initialPick(options: AnswerModelOption[], configured: string | null, recommended: string): string {
  const match = configured ? options.find((o) => sameModel(o.model, configured) && o.model.includes(":")) : undefined;
  return match?.model ?? recommended;
}
