/** R41.7: `/health` while the server is up but the database is not ready, plus the screens' wording. */
export type StartupStatus = {
  status: "starting" | "failed";
  phase: string;
  step: number;
  of: number;
  detail: string;
  started_at: string;
  upgrading: boolean;
  version: string;
  progress: { done: number; total: number; unit: "bytes" | "items" } | null;
  error?: string;
  data_state?: string;
  log_file?: string;
  support?: string;
};

export type GateState =
  | { kind: "checking" }
  | { kind: "ready" }
  | { kind: "unreachable" }
  | { kind: "starting" | "failed"; status: StartupStatus };

/** One probe of `/health` — 200 ready, 503 starting/failed, else unreachable. */
export async function probeStartup(fetcher: typeof fetch = fetch): Promise<GateState> {
  try {
    const res = await fetcher("/health", { headers: { "X-Anchor-Client": "ui" } });
    if (res.ok) return { kind: "ready" };
    if (res.status === 503) {
      const body = (await res.json()) as StartupStatus;
      if (body.status === "failed") return { kind: "failed", status: body };
      if (body.status === "starting") return { kind: "starting", status: body };
    }
    // A transient error (not ours): let the app load and show its own state.
    return { kind: "ready" };
  } catch {
    return { kind: "unreachable" };
  }
}

function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(n / 1e6))} MB`;
}

export function progressText(p: StartupStatus["progress"]): string | null {
  if (!p || p.total <= 0) return null;
  if (p.unit === "bytes") return `${formatBytes(p.done)} of ${formatBytes(p.total)} copied`;
  return `${p.done} of ${p.total} items`;
}

export function stepText(s: StartupStatus): string {
  const step = s.of > 1 && s.step > 0 ? `Step ${s.step} of ${s.of}: ` : "";
  return `${step}${s.detail}`;
}

