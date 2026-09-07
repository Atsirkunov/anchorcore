# Obsidian vault → AnchorCore

Keep your vault. Point AnchorCore at the folder. Same notes, better answers.

## Why

AI-on-Obsidian today means stuffing whole notes into the model's context
window (plugin, MCP dump, copy-paste): bloated, expensive, lossy, no
provenance. AnchorCore retrieves instead — hybrid `vec0 + FTS + RRF` with
graph hops, section-level citations. The model sees a handful of cited
chunks, not 40 files.

## Setup (3 steps, nothing to migrate)

1. Sources → Local folder → select your vault directory → Sync.
2. Ask — e.g. "what did I decide about X, and why?"
3. Every answer cites `file › section` — click through to the note.

The folder watcher picks up new/edited notes automatically. Data stays in
`~/.anchorcore`; the vault itself is only read.

## What works today / next

- Today: all markdown ingested as text — headings become the section tree,
  answers cite `§` / heading paths.
- Next: `[[wikilinks]]` → entity relationships, `#tags` → taxonomy,
  frontmatter → dates/metadata. Vault authors already structured the graph;
  ingest will consume it.
