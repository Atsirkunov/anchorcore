/** R41.7: `/health` while the server is up but the database is not ready, plus the screens' wording. */
export type StartupStatus = {
  status: "starting" | "failed";
  phase: string;
  step: number;
  of: number;
  detail: string;
  started_at: string;
  /** When the current step or its wording last changed (R41.10). */
  detail_since?: string;
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


/** "This step has been running for 3 min 12 s" once a phase passes 10 s, so a
 * long step without a progress bar still shows the app is working (R41.10). */
export function elapsedText(since: string | undefined, now: number = Date.now()): string | null {
  const start = since ? Date.parse(since) : NaN;
  if (Number.isNaN(start)) return null;
  const secs = Math.floor((now - start) / 1000);
  if (secs < 10) return null;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const span = h > 0 ? `${h} h ${m} min` : m > 0 ? `${m} min ${s} s` : `${s} s`;
  return `This step has been running for ${span}`;
}
