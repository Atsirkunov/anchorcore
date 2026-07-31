"use client";

import { useCallback, useEffect, useState } from "react";
import { listDocuments, listObjects, uploadDocument } from "../lib/api";
import type { DocumentSummary, KnowledgeObject } from "../lib/types";

const KIND_COLORS: Record<KnowledgeObject["kind"], string> = {
  decision: "#8b5cf6",
  document: "#3b82f6",
  action: "#f59e0b",
  note: "#6b7280",
};

export default function Home() {
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [objects, setObjects] = useState<KnowledgeObject[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setDocuments(await listDocuments());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const selectDocument = useCallback(async (id: number) => {
    setSelectedId(id);
    try {
      setObjects(await listObjects(id));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  async function onUpload(file: File | undefined) {
    if (!file) return;
    setUploading(true);
    setError(null);
    try {
      const doc = await uploadDocument(file);
      await refresh();
      await selectDocument(doc.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setUploading(false);
    }
  }

  return (
    <main style={{ maxWidth: 1080, margin: "0 auto", padding: "2rem 1rem" }}>
      <h1 style={{ fontSize: "1.6rem", margin: "0 0 0.25rem" }}>AnchorCore</h1>
      <p style={{ color: "#9ca3af", margin: "0 0 1.5rem" }}>
        Upload a file to extract decisions, actions, and knowledge objects.
      </p>

      <label
        style={{
          display: "block",
          padding: "1.5rem",
          border: "2px dashed #374151",
          borderRadius: 12,
          textAlign: "center",
          cursor: "pointer",
          background: "#171a21",
        }}
      >
        {uploading ? "Classifying…" : "Click to upload a file (.txt, .md, .pdf, .html)"}
        <input type="file" hidden onChange={(e) => onUpload(e.target.files?.[0])} />
      </label>

      {error && <p style={{ color: "#f87171", marginTop: "1rem" }}>{error}</p>}

      <div style={{ display: "grid", gridTemplateColumns: "300px 1fr", gap: "1.5rem", marginTop: "2rem" }}>
        <aside>
          <h2 style={{ fontSize: "1rem", color: "#9ca3af", textTransform: "uppercase" }}>Documents</h2>
          <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: "0.5rem" }}>
            {documents.map((doc) => (
              <li key={doc.id}>
                <button
                  onClick={() => selectDocument(doc.id)}
                  style={{
                    width: "100%",
                    textAlign: "left",
                    padding: "0.6rem 0.75rem",
                    borderRadius: 8,
                    border: selectedId === doc.id ? "1px solid #6366f1" : "1px solid #2d333b",
                    background: selectedId === doc.id ? "#1e2130" : "#171a21",
                    color: "#e6e8eb",
                    cursor: "pointer",
                  }}
                >
                  <div style={{ fontWeight: 600 }}>{doc.filename}</div>
                  <div style={{ fontSize: "0.75rem", color: "#9ca3af" }}>
                    {doc.text_length.toLocaleString()} chars · {new Date(doc.created_at).toLocaleDateString()}
                  </div>
                </button>
              </li>
            ))}
            {documents.length === 0 && <li style={{ color: "#6b7280" }}>No documents yet</li>}
          </ul>
        </aside>

        <section>
          <h2 style={{ fontSize: "1rem", color: "#9ca3af", textTransform: "uppercase" }}>
            Knowledge objects{selectedId === null ? "" : ` — ${documents.find((d) => d.id === selectedId)?.filename ?? ""}`}
          </h2>
          <div style={{ display: "grid", gap: "0.75rem" }}>
            {objects.map((obj) => (
              <article
                key={obj.id}
                style={{
                  border: "1px solid #2d333b",
                  borderLeft: `4px solid ${KIND_COLORS[obj.kind]}`,
                  borderRadius: 8,
                  padding: "0.75rem 1rem",
                  background: "#171a21",
                }}
              >
                <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "0.25rem" }}>
                  <span
                    style={{
                      fontSize: "0.7rem",
                      textTransform: "uppercase",
                      background: KIND_COLORS[obj.kind],
                      color: "#0f1115",
                      padding: "0.15rem 0.5rem",
                      borderRadius: 999,
                      fontWeight: 700,
                    }}
                  >
                    {obj.kind}
                  </span>
                  <span style={{ fontSize: "0.75rem", color: "#9ca3af" }}>confidence {(obj.confidence * 100).toFixed(0)}%</span>
                </div>
                <p style={{ margin: "0.25rem 0", fontWeight: 600 }}>{obj.summary}</p>
                {obj.reasoning && <p style={{ margin: "0.25rem 0", color: "#9ca3af", fontSize: "0.85rem" }}>{obj.reasoning}</p>}
                <div style={{ fontSize: "0.75rem", color: "#6b7280" }}>
                  source: {obj.source} {obj.author && `· author: ${obj.author}`}
                </div>
              </article>
            ))}
            {selectedId !== null && objects.length === 0 && (
              <p style={{ color: "#6b7280" }}>No knowledge objects extracted.</p>
            )}
            {selectedId === null && <p style={{ color: "#6b7280" }}>Select a document to see extracted objects.</p>}
          </div>
        </section>
      </div>
    </main>
  );
}
