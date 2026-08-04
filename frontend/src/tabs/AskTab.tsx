import { useState } from "react";
import { api } from "../api";
import type { AskResponse } from "../types";

export function AskTab() {
  const [question, setQuestion] = useState("");
  const [result, setResult] = useState<AskResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    if (!question.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await api.ask(question));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <h2>Ask your memory</h2>
      <div style={{ display: "flex", gap: 8 }}>
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && !busy && submit()}
          placeholder="What was decided about X, and why?"
          style={{ flex: 1, padding: "0.6rem 0.8rem", borderRadius: 8, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb" }}
        />
        <button onClick={submit} disabled={busy} style={{ padding: "0.6rem 1.2rem", borderRadius: 8, border: "none", background: "#6366f1", color: "#fff", cursor: busy ? "not-allowed" : "pointer", opacity: busy ? 0.6 : 1 }}>
          {busy ? "Thinking…" : "Ask"}
        </button>
      </div>
      {busy && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 12, color: "#9ca3af", fontSize: 13 }}>
          <span style={styles.spinner} />
          Retrieving context and composing an answer…
        </div>
      )}
      {error && <p style={{ color: "#f87171" }}>{error}</p>}
      {result && (
        <div style={{ marginTop: 16 }}>
          <div style={{ whiteSpace: "pre-wrap", background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "1rem", lineHeight: 1.6 }}>
            {result.answer}
          </div>
          {result.citations.length > 0 && (
            <div style={{ marginTop: 12 }}>
              <div style={{ fontSize: 12, color: "#9ca3af", textTransform: "uppercase" }}>Sources</div>
              {result.citations.map((c, i) => (
                <div key={i} style={{ marginTop: 6, padding: "0.5rem 0.75rem", background: "#14171d", borderRadius: 6, fontSize: 13 }}>
                  <span style={{ color: "#8b5cf6" }}>[{c.kind}]</span> {c.summary}
                  <div style={{ color: "#6b7280", fontSize: 12 }}>{c.source_ref} · score {c.score}</div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  spinner: {
    width: 14,
    height: 14,
    borderRadius: "50%",
    border: "2px solid #2d333b",
    borderTopColor: "#7dd3fc",
    animation: "spin 0.8s linear infinite",
    flexShrink: 0,
  },
};
